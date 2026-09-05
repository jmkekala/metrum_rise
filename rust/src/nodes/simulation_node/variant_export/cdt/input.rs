// SPDX-License-Identifier: GPL-2.0-only

//! Terrain CDT input, windowing, fingerprint, and sampling helpers.

use super::super::super::*;
use crate::simulation::core::round_f64_to_i64;
use crate::simulation::terrain::cdt::{
    TerrainCdtTieInGuideConstraint, TerrainCdtTieInGuideSample,
    clip_terrain_cdt_road_loop_to_patch, clip_terrain_cdt_segment_to_patch,
};
use std::collections::{BTreeSet, HashSet};

const TERRAIN_CDT_MIN_WINDOW_EXTENT_M: f32 = 0.001;
// Two 32 m road-query cells per CDT tile bound each independently cached road rebuild.
const TERRAIN_CDT_TILE_SPAN_MM: i64 = 64_000;
const TERRAIN_CDT_TILE_HALO_M: f32 = 64.0;
pub(in crate::nodes::simulation_node) const TERRAIN_CDT_TILE_NEIGHBORS: [(i64, i64); 4] =
    [(-1, 0), (0, -1), (0, 1), (1, 0)];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::nodes::simulation_node) struct TerrainCdtTileId {
    pub(in crate::nodes::simulation_node) x: i64,
    pub(in crate::nodes::simulation_node) z: i64,
}

struct TerrainCdtTileContributor {
    road_loop: TerrainCdtRoadLoop,
    influence_bounds: (f32, f32, f32, f32),
    guide_samples: Vec<TerrainCdtTieInGuideSample>,
    guide_constraints: Vec<TerrainCdtTieInGuideConstraint>,
}

struct TerrainCdtTileLoop {
    source_group_id: u64,
    road_loop: TerrainCdtRoadLoop,
}

struct TerrainCdtPlannedWindow {
    tile_id: TerrainCdtTileId,
    key: RefinedTerrainCdtWindowKey,
    cdt_input: TerrainCdtInput,
    matching_previous: Option<Arc<CachedRefinedTerrainCdtWindow>>,
    has_engineered_contributor: bool,
    road_clip_fingerprints: Vec<u64>,
    site_clip_fingerprints: Vec<u64>,
}

/// Complete fixed-tile plan and its unique authoritative loop coverage.
pub(in crate::nodes::simulation_node) struct TerrainCdtWindowBuildPlan {
    /// Current-generation tiles, including reusable previous windows.
    pub(in crate::nodes::simulation_node) windows: Vec<RefinedTerrainCdtWindowBuildInput>,
    /// Previous windows carried directly without assembling a new input.
    pub(in crate::nodes::simulation_node) reused_windows: Vec<Arc<CachedRefinedTerrainCdtWindow>>,
    /// Unique queried loops whose exact influence intersects this render patch.
    pub(in crate::nodes::simulation_node) represented_road_loop_count: usize,
    /// Queried loops found only through the padded patch margin.
    pub(in crate::nodes::simulation_node) omitted_margin_loop_count: usize,
    /// Stable road-contributor ids independently expected from this plan.
    pub(in crate::nodes::simulation_node) expected_road_clip_fingerprints: Vec<u64>,
    /// Stable building-site contributor ids independently expected from this plan.
    pub(in crate::nodes::simulation_node) expected_site_clip_fingerprints: Vec<u64>,
}

impl SimulationNode {
    pub(in crate::nodes::simulation_node) fn terrain_cdt_window_build_inputs(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        road_loops: &[TerrainCdtRoadLoop],
        render_step_m: f32,
        site_grading: Option<TerrainCdtSiteGradingContext<'_>>,
        previous: Option<&CachedRefinedTerrainPatch>,
    ) -> TerrainCdtWindowBuildPlan {
        if road_loops.is_empty() {
            return TerrainCdtWindowBuildPlan {
                windows: Vec::new(),
                reused_windows: Vec::new(),
                represented_road_loop_count: 0,
                omitted_margin_loop_count: 0,
                expected_road_clip_fingerprints: Vec::new(),
                expected_site_clip_fingerprints: Vec::new(),
            };
        }
        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        let previous_windows = previous
            .map(|cached| {
                cached
                    .windows
                    .iter()
                    .map(|window| {
                        (
                            Self::terrain_cdt_window_spatial_key(window.key),
                            Arc::clone(window),
                        )
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();

        let mut contributors = Vec::new();
        for road_loop in road_loops {
            let Some(influence_bounds) = Self::terrain_cdt_local_sample_bounds(
                terrain,
                patch,
                std::slice::from_ref(road_loop),
                safe_render_step_m,
            ) else {
                continue;
            };
            let mut guide_samples = Vec::new();
            let mut guide_constraints = Vec::new();
            let mut sample_keys = HashSet::new();
            RoadSurfaceSystem::append_terrain_cdt_roadbed_grading_envelope(
                terrain,
                std::slice::from_ref(road_loop),
                safe_render_step_m,
                &mut guide_samples,
                &mut guide_constraints,
                &mut sample_keys,
            );
            contributors.push(TerrainCdtTileContributor {
                road_loop: road_loop.clone(),
                influence_bounds,
                guide_samples,
                guide_constraints,
            });
        }

        let mut contributors_by_tile = BTreeMap::<TerrainCdtTileId, Vec<usize>>::new();
        let mut represented_road_loop_count = 0;
        for (contributor_index, contributor) in contributors.iter().enumerate() {
            let tile_ids =
                Self::terrain_cdt_tile_ids_for_bounds(patch, contributor.influence_bounds);
            if !tile_ids.is_empty() {
                represented_road_loop_count += 1;
            }
            for tile_id in tile_ids {
                contributors_by_tile
                    .entry(tile_id)
                    .or_default()
                    .push(contributor_index);
            }
        }
        let mut expected_road_clip_fingerprints = BTreeSet::new();
        let mut expected_site_clip_fingerprints = BTreeSet::new();
        let current_coverage = contributors_by_tile
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        let mut planned_tiles = current_coverage.clone();
        for tile_id in &current_coverage {
            for (offset_x, offset_z) in TERRAIN_CDT_TILE_NEIGHBORS {
                let neighbor = TerrainCdtTileId {
                    x: tile_id.x + offset_x,
                    z: tile_id.z + offset_z,
                };
                if Self::terrain_cdt_tile_bounds(patch, neighbor).is_some() {
                    planned_tiles.insert(neighbor);
                }
            }
        }

        let mut planned_windows = Vec::with_capacity(planned_tiles.len());
        let mut current_spatial_keys = BTreeSet::new();
        for tile_id in planned_tiles {
            let Some(bounds) = Self::terrain_cdt_tile_bounds(patch, tile_id) else {
                continue;
            };
            let contributor_indices = contributors_by_tile
                .get(&tile_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let halo_patch = TerrainCdtPatch::new(
                f64::from(bounds.0 - TERRAIN_CDT_TILE_HALO_M),
                f64::from(bounds.1 - TERRAIN_CDT_TILE_HALO_M),
                f64::from(bounds.2 + TERRAIN_CDT_TILE_HALO_M),
                f64::from(bounds.3 + TERRAIN_CDT_TILE_HALO_M),
                [0.0; 4],
            );
            let (local_road_loops, road_clip_fingerprints, site_clip_fingerprints) =
                Self::terrain_cdt_tile_local_loops_and_manifest(
                    tile_id,
                    halo_patch,
                    &contributors,
                    contributor_indices,
                );
            expected_road_clip_fingerprints.extend(road_clip_fingerprints.iter().copied());
            expected_site_clip_fingerprints.extend(site_clip_fingerprints.iter().copied());

            let core_patch = TerrainCdtPatch::new(
                bounds.0.into(),
                bounds.1.into(),
                bounds.2.into(),
                bounds.3.into(),
                [0.0; 4],
            );
            let mut guide_samples = contributor_indices
                .iter()
                .flat_map(|&index| contributors[index].guide_samples.iter().copied())
                .filter(|sample| Self::terrain_cdt_vertex_in_bounds(sample.vertex, bounds))
                .collect::<Vec<_>>();
            let guide_constraints = if contributor_indices.len() == 1 && local_road_loops.len() == 1
            {
                contributors[contributor_indices[0]]
                    .guide_constraints
                    .iter()
                    .filter_map(|constraint| {
                        clip_terrain_cdt_segment_to_patch(
                            constraint.start,
                            constraint.end,
                            core_patch,
                        )
                        .map(|(start, end)| {
                            guide_samples.push(TerrainCdtTieInGuideSample { vertex: start });
                            guide_samples.push(TerrainCdtTieInGuideSample { vertex: end });
                            TerrainCdtTieInGuideConstraint { start, end }
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            Self::sort_dedup_terrain_cdt_guide_samples(&mut guide_samples);
            let cdt_input = Self::terrain_cdt_input_for_bounds_with_guides(
                terrain,
                patch,
                local_road_loops,
                safe_render_step_m,
                bounds,
                guide_samples,
                guide_constraints,
                site_grading,
            );
            let key = Self::terrain_cdt_window_key(&cdt_input);
            let spatial_key = Self::terrain_cdt_window_spatial_key(key);
            current_spatial_keys.insert(spatial_key);
            planned_windows.push(TerrainCdtPlannedWindow {
                tile_id,
                key,
                cdt_input,
                matching_previous: previous_windows.get(&spatial_key).cloned(),
                has_engineered_contributor: !contributor_indices.is_empty(),
                road_clip_fingerprints,
                site_clip_fingerprints,
            });
        }

        let mut changed_tiles = planned_windows
            .iter()
            .filter(|window| {
                window
                    .matching_previous
                    .as_ref()
                    .is_none_or(|previous| previous.key.fingerprint != window.key.fingerprint)
            })
            .map(|window| window.tile_id)
            .collect::<BTreeSet<_>>();
        for (spatial_key, previous) in &previous_windows {
            if !current_spatial_keys.contains(spatial_key) {
                changed_tiles.insert(Self::terrain_cdt_tile_id_for_window_key(previous.key));
            }
        }
        let mut forced_rebuild = changed_tiles.clone();
        for tile_id in changed_tiles {
            for (offset_x, offset_z) in TERRAIN_CDT_TILE_NEIGHBORS {
                forced_rebuild.insert(TerrainCdtTileId {
                    x: tile_id.x + offset_x,
                    z: tile_id.z + offset_z,
                });
            }
        }
        let windows = planned_windows
            .into_iter()
            .map(|window| {
                let previous = (!forced_rebuild.contains(&window.tile_id))
                    .then(|| Self::terrain_cdt_reusable_previous(&window))
                    .flatten();
                RefinedTerrainCdtWindowBuildInput {
                    key: window.key,
                    cdt_input: window.cdt_input,
                    previous,
                    has_engineered_contributor: window.has_engineered_contributor,
                    road_clip_fingerprints: window.road_clip_fingerprints,
                    site_clip_fingerprints: window.site_clip_fingerprints,
                }
            })
            .collect();
        TerrainCdtWindowBuildPlan {
            windows,
            reused_windows: Vec::new(),
            represented_road_loop_count,
            omitted_margin_loop_count: road_loops.len().saturating_sub(represented_road_loop_count),
            expected_road_clip_fingerprints: expected_road_clip_fingerprints.into_iter().collect(),
            expected_site_clip_fingerprints: expected_site_clip_fingerprints.into_iter().collect(),
        }
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_incremental_window_build_inputs(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        graph: &crate::simulation::network::graph::RegionGraph,
        road_surface: &RoadSurfaceSystem,
        sites: &BuildingSiteTerrainSnapshot,
        tile_keys: &[(i64, i64)],
        render_step_m: f32,
        clip_query_margin_m: f32,
        previous: &CachedRefinedTerrainPatch,
    ) -> (TerrainCdtWindowBuildPlan, RoadClipLoopQuery) {
        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        let query_margin_m = clip_query_margin_m.max(TERRAIN_CDT_TILE_HALO_M);
        let rebuild_tiles = tile_keys
            .iter()
            .map(|&(x, z)| TerrainCdtTileId { x, z })
            .filter(|tile_id| Self::terrain_cdt_tile_bounds(patch, *tile_id).is_some())
            .collect::<BTreeSet<_>>();
        let previous_by_tile = previous
            .windows
            .iter()
            .map(|window| {
                (
                    Self::terrain_cdt_tile_id_for_window_key(window.key),
                    Arc::clone(window),
                )
            })
            .collect::<BTreeMap<_, _>>();

        // Neighboring dirty tiles have heavily overlapping query halos. Union their road
        // footprints once, then distribute exact grading influence back to each tile.
        let mut group_queries = Self::terrain_cdt_connected_tile_groups(&rebuild_tiles)
            .into_par_iter()
            .filter_map(|tile_ids| {
                let bounds = Self::terrain_cdt_tile_group_bounds(patch, &tile_ids)?;
                let query = Self::road_clip_loop_query_for_snapshot(
                    graph,
                    road_surface,
                    sites,
                    bounds.0 - query_margin_m,
                    bounds.1 - query_margin_m,
                    bounds.2 + query_margin_m,
                    bounds.3 + query_margin_m,
                );
                let query_counts = (
                    query.source_count,
                    query.road_source_count,
                    query.road_loop_count,
                    query.site_loop_count,
                    query.clip_error_label,
                );
                let contributors = query
                    .cdt_road_loops
                    .into_iter()
                    .filter_map(|road_loop| {
                        let influence_bounds = Self::terrain_cdt_local_sample_bounds(
                            terrain,
                            patch,
                            std::slice::from_ref(&road_loop),
                            safe_render_step_m,
                        )?;
                        let mut guide_samples = Vec::new();
                        let mut guide_constraints = Vec::new();
                        let mut sample_keys = HashSet::new();
                        RoadSurfaceSystem::append_terrain_cdt_roadbed_grading_envelope(
                            terrain,
                            std::slice::from_ref(&road_loop),
                            safe_render_step_m,
                            &mut guide_samples,
                            &mut guide_constraints,
                            &mut sample_keys,
                        );
                        Some(TerrainCdtTileContributor {
                            road_loop,
                            influence_bounds,
                            guide_samples,
                            guide_constraints,
                        })
                    })
                    .collect::<Vec<_>>();
                Some((tile_ids, contributors, query_counts))
            })
            .collect::<Vec<_>>();
        group_queries.sort_by_key(|(tile_ids, _, _)| tile_ids.first().copied());

        let mut contributors = Vec::new();
        let mut contributor_indices_by_tile = BTreeMap::<TerrainCdtTileId, Vec<usize>>::new();
        let mut source_count = 0usize;
        let mut road_source_count = 0usize;
        let mut road_loop_count = 0usize;
        let mut site_loop_count = 0usize;
        let mut represented_road_loop_count = 0usize;
        let mut clip_error_label = None;
        for (tile_ids, group_contributors, query_counts) in group_queries {
            source_count = source_count.saturating_add(query_counts.0);
            road_source_count = road_source_count.saturating_add(query_counts.1);
            road_loop_count = road_loop_count.saturating_add(query_counts.2);
            site_loop_count = site_loop_count.saturating_add(query_counts.3);
            clip_error_label = clip_error_label.or(query_counts.4);
            for contributor in group_contributors {
                let affected_tiles =
                    Self::terrain_cdt_tile_ids_for_bounds(patch, contributor.influence_bounds)
                        .into_iter()
                        .filter(|tile_id| tile_ids.binary_search(tile_id).is_ok())
                        .collect::<Vec<_>>();
                if affected_tiles.is_empty() {
                    continue;
                }
                let contributor_index = contributors.len();
                contributors.push(contributor);
                represented_road_loop_count = represented_road_loop_count.saturating_add(1);
                for tile_id in affected_tiles {
                    contributor_indices_by_tile
                        .entry(tile_id)
                        .or_default()
                        .push(contributor_index);
                }
            }
        }

        let current_contributor_tiles = contributor_indices_by_tile
            .iter()
            .filter_map(|(&tile_id, indices)| (!indices.is_empty()).then_some(tile_id))
            .collect::<BTreeSet<_>>();
        let retained_previous_contributor_tiles = previous_by_tile
            .iter()
            .filter_map(|(&tile_id, window)| {
                (!rebuild_tiles.contains(&tile_id) && window.has_engineered_contributor)
                    .then_some(tile_id)
            })
            .collect::<BTreeSet<_>>();
        let mut expected_road_clip_fingerprints = previous_by_tile
            .iter()
            .filter(|(tile_id, _)| !rebuild_tiles.contains(tile_id))
            .flat_map(|(_, window)| window.road_clip_fingerprints.iter().copied())
            .collect::<BTreeSet<_>>();
        let mut expected_site_clip_fingerprints = previous_by_tile
            .iter()
            .filter(|(tile_id, _)| !rebuild_tiles.contains(tile_id))
            .flat_map(|(_, window)| window.site_clip_fingerprints.iter().copied())
            .collect::<BTreeSet<_>>();
        let mut included_rebuild_tiles = current_contributor_tiles.clone();
        for contributor_tile in current_contributor_tiles
            .iter()
            .chain(&retained_previous_contributor_tiles)
        {
            for (offset_x, offset_z) in TERRAIN_CDT_TILE_NEIGHBORS {
                let neighbor = TerrainCdtTileId {
                    x: contributor_tile.x + offset_x,
                    z: contributor_tile.z + offset_z,
                };
                if rebuild_tiles.contains(&neighbor) {
                    included_rebuild_tiles.insert(neighbor);
                }
            }
        }

        let mut windows = Vec::with_capacity(included_rebuild_tiles.len());
        for tile_id in included_rebuild_tiles {
            let contributor_indices = contributor_indices_by_tile
                .remove(&tile_id)
                .unwrap_or_default();
            if let Some(window) = Self::terrain_cdt_planned_window_for_tile(
                terrain,
                patch,
                tile_id,
                &contributors,
                &contributor_indices,
                safe_render_step_m,
                Some(TerrainCdtSiteGradingContext {
                    source: TerrainCdtSiteGradingSource::Snapshot(sites),
                    graph,
                    road_surface,
                }),
                previous_by_tile.get(&tile_id).cloned(),
            ) {
                expected_road_clip_fingerprints
                    .extend(window.road_clip_fingerprints.iter().copied());
                expected_site_clip_fingerprints
                    .extend(window.site_clip_fingerprints.iter().copied());
                let previous = Self::terrain_cdt_reusable_previous(&window);
                windows.push(RefinedTerrainCdtWindowBuildInput {
                    key: window.key,
                    cdt_input: window.cdt_input,
                    previous,
                    has_engineered_contributor: window.has_engineered_contributor,
                    road_clip_fingerprints: window.road_clip_fingerprints,
                    site_clip_fingerprints: window.site_clip_fingerprints,
                });
            }
        }
        windows.sort_by_key(|window| window.key);
        let reused_windows = previous_by_tile
            .into_iter()
            .filter_map(|(tile_id, window)| (!rebuild_tiles.contains(&tile_id)).then_some(window))
            .collect::<Vec<_>>();
        (
            TerrainCdtWindowBuildPlan {
                windows,
                reused_windows,
                represented_road_loop_count,
                omitted_margin_loop_count: road_loop_count
                    .saturating_add(site_loop_count)
                    .saturating_sub(represented_road_loop_count),
                expected_road_clip_fingerprints: expected_road_clip_fingerprints
                    .into_iter()
                    .collect(),
                expected_site_clip_fingerprints: expected_site_clip_fingerprints
                    .into_iter()
                    .collect(),
            },
            RoadClipLoopQuery {
                cdt_road_loops: Vec::new(),
                source_count,
                road_source_count,
                road_loop_count,
                site_loop_count,
                clip_error_label,
            },
        )
    }

    fn terrain_cdt_connected_tile_groups(
        tile_ids: &BTreeSet<TerrainCdtTileId>,
    ) -> Vec<Vec<TerrainCdtTileId>> {
        let mut remaining = tile_ids.clone();
        let mut groups = Vec::new();
        while let Some(seed) = remaining.pop_first() {
            let mut pending = vec![seed];
            let mut group = Vec::new();
            while let Some(tile_id) = pending.pop() {
                group.push(tile_id);
                for offset_z in -1..=1 {
                    for offset_x in -1..=1 {
                        if offset_x == 0 && offset_z == 0 {
                            continue;
                        }
                        let neighbor = TerrainCdtTileId {
                            x: tile_id.x + offset_x,
                            z: tile_id.z + offset_z,
                        };
                        if remaining.remove(&neighbor) {
                            pending.push(neighbor);
                        }
                    }
                }
            }
            group.sort_unstable();
            groups.push(group);
        }
        groups
    }

    fn terrain_cdt_tile_group_bounds(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        tile_ids: &[TerrainCdtTileId],
    ) -> Option<(f32, f32, f32, f32)> {
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        for &tile_id in tile_ids {
            let tile = Self::terrain_cdt_tile_bounds(patch, tile_id)?;
            bounds = Some(bounds.map_or(tile, |current| {
                (
                    current.0.min(tile.0),
                    current.1.min(tile.1),
                    current.2.max(tile.2),
                    current.3.max(tile.3),
                )
            }));
        }
        bounds
    }

    fn terrain_cdt_reusable_previous(
        window: &TerrainCdtPlannedWindow,
    ) -> Option<Arc<CachedRefinedTerrainCdtWindow>> {
        window
            .matching_previous
            .as_ref()
            .filter(|previous| {
                Self::terrain_cdt_cached_window_matches_current_manifest(
                    previous,
                    window.key,
                    window.has_engineered_contributor,
                    &window.road_clip_fingerprints,
                    &window.site_clip_fingerprints,
                )
            })
            .cloned()
    }

    fn terrain_cdt_cached_window_matches_current_manifest(
        previous: &CachedRefinedTerrainCdtWindow,
        key: RefinedTerrainCdtWindowKey,
        has_engineered_contributor: bool,
        road_clip_fingerprints: &[u64],
        site_clip_fingerprints: &[u64],
    ) -> bool {
        previous.key == key
            && previous.has_engineered_contributor == has_engineered_contributor
            && previous.road_clip_fingerprints == road_clip_fingerprints
            && previous.site_clip_fingerprints == site_clip_fingerprints
    }

    fn terrain_cdt_planned_window_for_tile(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        tile_id: TerrainCdtTileId,
        contributors: &[TerrainCdtTileContributor],
        contributor_indices: &[usize],
        render_step_m: f32,
        site_grading: Option<TerrainCdtSiteGradingContext<'_>>,
        matching_previous: Option<Arc<CachedRefinedTerrainCdtWindow>>,
    ) -> Option<TerrainCdtPlannedWindow> {
        let bounds = Self::terrain_cdt_tile_bounds(patch, tile_id)?;
        let halo_patch = TerrainCdtPatch::new(
            f64::from(bounds.0 - TERRAIN_CDT_TILE_HALO_M),
            f64::from(bounds.1 - TERRAIN_CDT_TILE_HALO_M),
            f64::from(bounds.2 + TERRAIN_CDT_TILE_HALO_M),
            f64::from(bounds.3 + TERRAIN_CDT_TILE_HALO_M),
            [0.0; 4],
        );
        let (local_road_loops, road_clip_fingerprints, site_clip_fingerprints) =
            Self::terrain_cdt_tile_local_loops_and_manifest(
                tile_id,
                halo_patch,
                contributors,
                contributor_indices,
            );
        let core_patch = TerrainCdtPatch::new(
            bounds.0.into(),
            bounds.1.into(),
            bounds.2.into(),
            bounds.3.into(),
            [0.0; 4],
        );
        let mut guide_samples = contributor_indices
            .iter()
            .flat_map(|&index| contributors[index].guide_samples.iter().copied())
            .filter(|sample| Self::terrain_cdt_vertex_in_bounds(sample.vertex, bounds))
            .collect::<Vec<_>>();
        let guide_constraints = if contributor_indices.len() == 1 && local_road_loops.len() == 1 {
            contributors[contributor_indices[0]]
                .guide_constraints
                .iter()
                .filter_map(|constraint| {
                    clip_terrain_cdt_segment_to_patch(constraint.start, constraint.end, core_patch)
                        .map(|(start, end)| {
                            guide_samples.push(TerrainCdtTieInGuideSample { vertex: start });
                            guide_samples.push(TerrainCdtTieInGuideSample { vertex: end });
                            TerrainCdtTieInGuideConstraint { start, end }
                        })
                })
                .collect()
        } else {
            Vec::new()
        };
        Self::sort_dedup_terrain_cdt_guide_samples(&mut guide_samples);
        let cdt_input = Self::terrain_cdt_input_for_bounds_with_guides(
            terrain,
            patch,
            local_road_loops,
            render_step_m,
            bounds,
            guide_samples,
            guide_constraints,
            site_grading,
        );
        let key = Self::terrain_cdt_window_key(&cdt_input);
        Some(TerrainCdtPlannedWindow {
            tile_id,
            key,
            cdt_input,
            matching_previous,
            has_engineered_contributor: !contributor_indices.is_empty(),
            road_clip_fingerprints,
            site_clip_fingerprints,
        })
    }

    #[cfg(test)]
    pub(in crate::nodes::simulation_node) fn terrain_cdt_input_for_bounds(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        road_loops: &[TerrainCdtRoadLoop],
        render_step_m: f32,
        bounds: (f32, f32, f32, f32),
        site_grading: Option<TerrainCdtSiteGradingContext<'_>>,
    ) -> TerrainCdtInput {
        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        let mut guide_samples = Vec::new();
        let mut guide_constraints = Vec::new();
        let mut sample_keys = HashSet::new();
        RoadSurfaceSystem::append_terrain_cdt_roadbed_grading_envelope(
            terrain,
            road_loops,
            safe_render_step_m,
            &mut guide_samples,
            &mut guide_constraints,
            &mut sample_keys,
        );
        Self::terrain_cdt_input_for_bounds_with_guides(
            terrain,
            patch,
            road_loops.to_vec(),
            safe_render_step_m,
            bounds,
            guide_samples,
            guide_constraints,
            site_grading,
        )
    }

    fn terrain_cdt_input_for_bounds_with_guides(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        road_loops: Vec<TerrainCdtRoadLoop>,
        render_step_m: f32,
        bounds: (f32, f32, f32, f32),
        mut tie_in_guide_samples: Vec<TerrainCdtTieInGuideSample>,
        tie_in_guide_constraints: Vec<TerrainCdtTieInGuideConstraint>,
        site_grading: Option<TerrainCdtSiteGradingContext<'_>>,
    ) -> TerrainCdtInput {
        let (min_x, min_z, max_x, max_z) = bounds;
        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        let patch_model = Self::terrain_cdt_patch_for_bounds(terrain, min_x, min_z, max_x, max_z);
        let road_loops = road_loops
            .iter()
            .flat_map(|road_loop| clip_terrain_cdt_road_loop_to_patch(road_loop, patch_model))
            .collect();
        let mut source_samples = Vec::new();
        let mut sample_keys = tie_in_guide_samples
            .iter()
            .map(|sample| {
                (
                    round_f64_to_i64(sample.vertex.x * TERRAIN_CDT_SAMPLE_KEY_SCALE),
                    round_f64_to_i64(sample.vertex.z * TERRAIN_CDT_SAMPLE_KEY_SCALE),
                )
            })
            .collect::<HashSet<_>>();
        // The roadbed guides and window sides retain render-step detail. The background grid only
        // carries the source terrain, so sampling it more densely than the terrain patch adds no
        // information and multiplies CDT insertion and face-classification work.
        let background_step_m = Self::regular_terrain_mesh_step_m(patch).max(safe_render_step_m);
        let grid_step_m =
            Self::terrain_cdt_grid_sample_step_m(min_x, min_z, max_x, max_z, background_step_m);
        if let Some(site_grading) = site_grading {
            site_grading.append_guides(
                terrain,
                (min_x, min_z, max_x, max_z),
                safe_render_step_m,
                &mut tie_in_guide_samples,
                &mut sample_keys,
            );
        }
        Self::append_terrain_cdt_grid_samples(
            terrain,
            patch,
            min_x,
            min_z,
            max_x,
            max_z,
            grid_step_m,
            &mut source_samples,
            &mut sample_keys,
        );
        Self::append_terrain_cdt_window_boundary_samples(
            terrain,
            min_x,
            min_z,
            max_x,
            max_z,
            safe_render_step_m,
            &mut source_samples,
            &mut sample_keys,
        );
        TerrainCdtInput::new(patch_model, road_loops, source_samples)
            .with_tie_in_guide_samples(tie_in_guide_samples)
            .with_tie_in_guide_constraints(tie_in_guide_constraints)
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_tile_ids_for_bounds(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        bounds: (f32, f32, f32, f32),
    ) -> Vec<TerrainCdtTileId> {
        let patch_min_x_mm = Self::quantize_cdt_coord_mm(f64::from(patch.world_origin_x));
        let patch_min_z_mm = Self::quantize_cdt_coord_mm(f64::from(patch.world_origin_z));
        let patch_max_x_mm =
            Self::quantize_cdt_coord_mm(f64::from(patch.world_origin_x + patch.world_size_x));
        let patch_max_z_mm =
            Self::quantize_cdt_coord_mm(f64::from(patch.world_origin_z + patch.world_size_z));
        let min_x_mm = Self::quantize_cdt_coord_mm(f64::from(bounds.0)).max(patch_min_x_mm);
        let min_z_mm = Self::quantize_cdt_coord_mm(f64::from(bounds.1)).max(patch_min_z_mm);
        let max_x_mm = Self::quantize_cdt_coord_mm(f64::from(bounds.2)).min(patch_max_x_mm);
        let max_z_mm = Self::quantize_cdt_coord_mm(f64::from(bounds.3)).min(patch_max_z_mm);
        if max_x_mm <= min_x_mm || max_z_mm <= min_z_mm {
            return Vec::new();
        }
        let start_x = min_x_mm.div_euclid(TERRAIN_CDT_TILE_SPAN_MM);
        let start_z = min_z_mm.div_euclid(TERRAIN_CDT_TILE_SPAN_MM);
        let end_x = (max_x_mm - 1).div_euclid(TERRAIN_CDT_TILE_SPAN_MM);
        let end_z = (max_z_mm - 1).div_euclid(TERRAIN_CDT_TILE_SPAN_MM);
        let mut ids = Vec::new();
        for z in start_z..=end_z {
            for x in start_x..=end_x {
                ids.push(TerrainCdtTileId { x, z });
            }
        }
        ids
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_tile_bounds(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        tile_id: TerrainCdtTileId,
    ) -> Option<(f32, f32, f32, f32)> {
        let patch_min_x_mm = Self::quantize_cdt_coord_mm(f64::from(patch.world_origin_x));
        let patch_min_z_mm = Self::quantize_cdt_coord_mm(f64::from(patch.world_origin_z));
        let patch_max_x_mm =
            Self::quantize_cdt_coord_mm(f64::from(patch.world_origin_x + patch.world_size_x));
        let patch_max_z_mm =
            Self::quantize_cdt_coord_mm(f64::from(patch.world_origin_z + patch.world_size_z));
        let tile_min_x_mm = tile_id.x.saturating_mul(TERRAIN_CDT_TILE_SPAN_MM);
        let tile_min_z_mm = tile_id.z.saturating_mul(TERRAIN_CDT_TILE_SPAN_MM);
        let min_x_mm = tile_min_x_mm.max(patch_min_x_mm);
        let min_z_mm = tile_min_z_mm.max(patch_min_z_mm);
        let max_x_mm = tile_min_x_mm
            .saturating_add(TERRAIN_CDT_TILE_SPAN_MM)
            .min(patch_max_x_mm);
        let max_z_mm = tile_min_z_mm
            .saturating_add(TERRAIN_CDT_TILE_SPAN_MM)
            .min(patch_max_z_mm);
        (max_x_mm > min_x_mm && max_z_mm > min_z_mm).then_some((
            min_x_mm as f32 / 1000.0,
            min_z_mm as f32 / 1000.0,
            max_x_mm as f32 / 1000.0,
            max_z_mm as f32 / 1000.0,
        ))
    }

    fn terrain_cdt_window_spatial_key(key: RefinedTerrainCdtWindowKey) -> (i64, i64, i64, i64) {
        (key.min_x_mm, key.min_z_mm, key.max_x_mm, key.max_z_mm)
    }

    fn terrain_cdt_tile_id_for_window_key(key: RefinedTerrainCdtWindowKey) -> TerrainCdtTileId {
        TerrainCdtTileId {
            x: key.min_x_mm.div_euclid(TERRAIN_CDT_TILE_SPAN_MM),
            z: key.min_z_mm.div_euclid(TERRAIN_CDT_TILE_SPAN_MM),
        }
    }

    fn terrain_cdt_vertex_in_bounds(
        vertex: TerrainCdtVertex,
        bounds: (f32, f32, f32, f32),
    ) -> bool {
        vertex.x >= f64::from(bounds.0) - 0.001
            && vertex.x <= f64::from(bounds.2) + 0.001
            && vertex.z >= f64::from(bounds.1) - 0.001
            && vertex.z <= f64::from(bounds.3) + 0.001
    }

    fn sort_dedup_terrain_cdt_guide_samples(samples: &mut Vec<TerrainCdtTieInGuideSample>) {
        samples.sort_by_key(|sample| {
            (
                Self::quantize_cdt_coord_mm(sample.vertex.x),
                Self::quantize_cdt_coord_mm(sample.vertex.z),
                sample.vertex.height_m.to_bits(),
            )
        });
        samples.dedup_by(|left, right| {
            Self::quantize_cdt_coord_mm(left.vertex.x)
                == Self::quantize_cdt_coord_mm(right.vertex.x)
                && Self::quantize_cdt_coord_mm(left.vertex.z)
                    == Self::quantize_cdt_coord_mm(right.vertex.z)
        });
    }

    fn terrain_cdt_tile_local_loops_and_manifest(
        tile_id: TerrainCdtTileId,
        halo_patch: TerrainCdtPatch,
        contributors: &[TerrainCdtTileContributor],
        contributor_indices: &[usize],
    ) -> (Vec<TerrainCdtRoadLoop>, Vec<u64>, Vec<u64>) {
        let mut tile_loops = Vec::new();
        let mut road_fingerprints = BTreeSet::new();
        let mut site_fingerprints = BTreeSet::new();
        for &index in contributor_indices {
            let contributor = &contributors[index];
            let clipped = clip_terrain_cdt_road_loop_to_patch(&contributor.road_loop, halo_patch);
            if clipped.is_empty() {
                continue;
            }
            let is_site = Self::terrain_cdt_loop_is_site(&contributor.road_loop);
            let fingerprint =
                Self::terrain_cdt_contributor_manifest_fingerprint(tile_id, &clipped, is_site);
            if is_site {
                site_fingerprints.insert(fingerprint);
            } else {
                road_fingerprints.insert(fingerprint);
            }
            tile_loops.extend(clipped.into_iter().map(|road_loop| TerrainCdtTileLoop {
                source_group_id: contributor.road_loop.footprint_group_id,
                road_loop,
            }));
        }
        (
            Self::rekey_terrain_cdt_tile_loops(tile_id, tile_loops),
            road_fingerprints.into_iter().collect(),
            site_fingerprints.into_iter().collect(),
        )
    }

    fn terrain_cdt_loop_is_site(road_loop: &TerrainCdtRoadLoop) -> bool {
        road_loop.source_edges.first().is_some_and(|_| {
            road_loop.source_edges.iter().all(|source_edge| {
                matches!(
                    source_edge.source,
                    TerrainCdtRoadBoundarySource::BuildingSiteBoundary { .. }
                )
            })
        })
    }

    fn terrain_cdt_contributor_manifest_fingerprint(
        tile_id: TerrainCdtTileId,
        clipped_loops: &[TerrainCdtRoadLoop],
        is_site: bool,
    ) -> u64 {
        let mut clipped_fingerprints = clipped_loops
            .iter()
            .map(Self::terrain_cdt_local_loop_fingerprint)
            .collect::<Vec<_>>();
        clipped_fingerprints.sort_unstable();
        let mut contributor_fingerprint = 0xcbf2_9ce4_8422_2325_u64;
        Self::hash_u64(
            &mut contributor_fingerprint,
            clipped_fingerprints.len() as u64,
        );
        for clipped_fingerprint in clipped_fingerprints {
            Self::hash_u64(&mut contributor_fingerprint, clipped_fingerprint);
        }
        Self::terrain_cdt_local_topology_id(
            tile_id,
            contributor_fingerprint,
            if is_site { 4 } else { 3 },
        )
    }

    fn rekey_terrain_cdt_tile_loops(
        tile_id: TerrainCdtTileId,
        road_loops: Vec<TerrainCdtTileLoop>,
    ) -> Vec<TerrainCdtRoadLoop> {
        let mut loops_by_source_group = BTreeMap::<u64, Vec<TerrainCdtRoadLoop>>::new();
        for tile_loop in road_loops {
            loops_by_source_group
                .entry(tile_loop.source_group_id)
                .or_default()
                .push(tile_loop.road_loop);
        }
        let mut groups = loops_by_source_group
            .into_values()
            .map(|mut loops| {
                loops.sort_by_key(|road_loop| {
                    (
                        road_loop.is_hole,
                        Self::terrain_cdt_local_loop_fingerprint(road_loop),
                    )
                });
                let loop_fingerprints = loops
                    .iter()
                    .map(Self::terrain_cdt_local_loop_fingerprint)
                    .collect::<Vec<_>>();
                (loop_fingerprints, loops)
            })
            .collect::<Vec<_>>();
        groups.sort_by(|left, right| left.0.cmp(&right.0));

        let mut rekeyed = Vec::new();
        for (loop_fingerprints, loops) in groups {
            let mut group_fingerprint = 0xcbf2_9ce4_8422_2325_u64;
            Self::hash_u64(&mut group_fingerprint, loop_fingerprints.len() as u64);
            for loop_fingerprint in &loop_fingerprints {
                Self::hash_u64(&mut group_fingerprint, *loop_fingerprint);
            }
            let group_id = Self::terrain_cdt_local_topology_id(tile_id, group_fingerprint, 0);
            for (local_loop_index, (mut road_loop, loop_fingerprint)) in
                loops.into_iter().zip(loop_fingerprints).enumerate()
            {
                let local_loop_index = u32::try_from(local_loop_index).unwrap_or(u32::MAX);
                road_loop.footprint_group_id = group_id;
                road_loop.local_loop_index = local_loop_index;
                road_loop.stable_piece_id = Self::terrain_cdt_local_topology_id(
                    tile_id,
                    loop_fingerprint,
                    u64::from(local_loop_index) + 1,
                );
                rekeyed.push(road_loop);
            }
        }
        rekeyed
    }

    fn terrain_cdt_local_loop_fingerprint(road_loop: &TerrainCdtRoadLoop) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        Self::hash_u64(&mut hash, u64::from(road_loop.is_hole));
        Self::hash_u64(&mut hash, road_loop.vertices.len() as u64);
        for vertex in &road_loop.vertices {
            Self::hash_terrain_cdt_vertex(&mut hash, *vertex);
        }
        Self::hash_terrain_cdt_source_edges(&mut hash, road_loop);
        hash
    }

    fn terrain_cdt_local_topology_id(
        tile_id: TerrainCdtTileId,
        loop_fingerprint: u64,
        discriminator: u64,
    ) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        Self::hash_i64(&mut hash, tile_id.x);
        Self::hash_i64(&mut hash, tile_id.z);
        Self::hash_u64(&mut hash, loop_fingerprint);
        Self::hash_u64(&mut hash, discriminator);
        hash
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_grid_sample_step_m(
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        render_step_m: f32,
    ) -> f32 {
        let safe_step_m = render_step_m.max(f32::EPSILON);
        let width_m = (max_x - min_x).max(0.0);
        let height_m = (max_z - min_z).max(0.0);
        let sample_x = (width_m / safe_step_m).ceil() + 1.0;
        let sample_z = (height_m / safe_step_m).ceil() + 1.0;
        let estimated_samples = sample_x * sample_z;
        if estimated_samples <= TERRAIN_CDT_MAX_LOCAL_GRID_SAMPLES {
            return safe_step_m;
        }

        let scale = (estimated_samples / TERRAIN_CDT_MAX_LOCAL_GRID_SAMPLES).sqrt();
        (safe_step_m * scale).max(safe_step_m)
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_window_key(
        input: &TerrainCdtInput,
    ) -> RefinedTerrainCdtWindowKey {
        RefinedTerrainCdtWindowKey {
            min_x_mm: Self::quantize_cdt_coord_mm(input.patch.min_x),
            min_z_mm: Self::quantize_cdt_coord_mm(input.patch.min_z),
            max_x_mm: Self::quantize_cdt_coord_mm(input.patch.max_x),
            max_z_mm: Self::quantize_cdt_coord_mm(input.patch.max_z),
            fingerprint: Self::terrain_cdt_input_fingerprint(input),
        }
    }

    pub(in crate::nodes::simulation_node) fn quantize_cdt_coord_mm(value: f64) -> i64 {
        round_f64_to_i64(value * 1000.0)
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_input_fingerprint(
        input: &TerrainCdtInput,
    ) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        Self::hash_i64(&mut hash, TERRAIN_CDT_CONTRACT_REVISION);
        Self::hash_u64(&mut hash, input.patch.min_x.to_bits());
        Self::hash_u64(&mut hash, input.patch.min_z.to_bits());
        Self::hash_u64(&mut hash, input.patch.max_x.to_bits());
        Self::hash_u64(&mut hash, input.patch.max_z.to_bits());
        for corner_height_m in input.patch.corner_heights_m {
            Self::hash_u64(&mut hash, u64::from(corner_height_m.to_bits()));
        }

        let mut road_loop_hashes = input
            .road_loops
            .iter()
            .map(|road_loop| {
                let mut loop_hash = 0xcbf2_9ce4_8422_2325_u64;
                Self::hash_u64(&mut loop_hash, road_loop.stable_piece_id);
                Self::hash_u64(&mut loop_hash, road_loop.footprint_group_id);
                Self::hash_u64(&mut loop_hash, u64::from(road_loop.local_loop_index));
                Self::hash_u64(&mut loop_hash, u64::from(road_loop.is_hole));
                Self::hash_u64(&mut loop_hash, road_loop.vertices.len() as u64);
                for vertex in &road_loop.vertices {
                    Self::hash_terrain_cdt_vertex(&mut loop_hash, *vertex);
                }
                Self::hash_terrain_cdt_source_edges(&mut loop_hash, road_loop);
                loop_hash
            })
            .collect::<Vec<_>>();
        road_loop_hashes.sort_unstable();
        Self::hash_u64(&mut hash, road_loop_hashes.len() as u64);
        for loop_hash in road_loop_hashes {
            Self::hash_u64(&mut hash, loop_hash);
        }

        let mut guide_sample_hashes = input
            .tie_in_guide_samples
            .iter()
            .map(|sample| {
                let mut sample_hash = 0xcbf2_9ce4_8422_2325_u64;
                Self::hash_terrain_cdt_vertex(&mut sample_hash, sample.vertex);
                sample_hash
            })
            .collect::<Vec<_>>();
        guide_sample_hashes.sort_unstable();
        Self::hash_u64(&mut hash, guide_sample_hashes.len() as u64);
        for sample_hash in guide_sample_hashes {
            Self::hash_u64(&mut hash, sample_hash);
        }

        let mut guide_constraint_hashes = input
            .tie_in_guide_constraints
            .iter()
            .map(|constraint| {
                let mut start_hash = 0xcbf2_9ce4_8422_2325_u64;
                Self::hash_terrain_cdt_vertex(&mut start_hash, constraint.start);
                let mut end_hash = 0xcbf2_9ce4_8422_2325_u64;
                Self::hash_terrain_cdt_vertex(&mut end_hash, constraint.end);
                let mut constraint_hash = 0xcbf2_9ce4_8422_2325_u64;
                Self::hash_u64(&mut constraint_hash, start_hash.min(end_hash));
                Self::hash_u64(&mut constraint_hash, start_hash.max(end_hash));
                constraint_hash
            })
            .collect::<Vec<_>>();
        guide_constraint_hashes.sort_unstable();
        Self::hash_u64(&mut hash, guide_constraint_hashes.len() as u64);
        for constraint_hash in guide_constraint_hashes {
            Self::hash_u64(&mut hash, constraint_hash);
        }

        let mut source_sample_hashes = input
            .source_samples
            .iter()
            .map(|sample| {
                let mut sample_hash = 0xcbf2_9ce4_8422_2325_u64;
                Self::hash_terrain_cdt_vertex(&mut sample_hash, *sample);
                sample_hash
            })
            .collect::<Vec<_>>();
        source_sample_hashes.sort_unstable();
        Self::hash_u64(&mut hash, source_sample_hashes.len() as u64);
        for sample_hash in source_sample_hashes {
            Self::hash_u64(&mut hash, sample_hash);
        }
        hash
    }

    fn hash_terrain_cdt_source_edges(hash: &mut u64, road_loop: &TerrainCdtRoadLoop) {
        let mut source_edge_hashes = road_loop
            .source_edges
            .iter()
            .map(|source_edge| {
                let mut start_hash = 0xcbf2_9ce4_8422_2325_u64;
                Self::hash_terrain_cdt_vertex(&mut start_hash, source_edge.start);
                let mut end_hash = 0xcbf2_9ce4_8422_2325_u64;
                Self::hash_terrain_cdt_vertex(&mut end_hash, source_edge.end);
                let mut source_edge_hash = 0xcbf2_9ce4_8422_2325_u64;
                Self::hash_u64(&mut source_edge_hash, start_hash.min(end_hash));
                Self::hash_u64(&mut source_edge_hash, start_hash.max(end_hash));
                Self::hash_terrain_cdt_boundary_source(&mut source_edge_hash, source_edge.source);
                source_edge_hash
            })
            .collect::<Vec<_>>();
        source_edge_hashes.sort_unstable();
        Self::hash_u64(hash, source_edge_hashes.len() as u64);
        for source_edge_hash in source_edge_hashes {
            Self::hash_u64(hash, source_edge_hash);
        }
    }

    fn hash_terrain_cdt_vertex(hash: &mut u64, vertex: TerrainCdtVertex) {
        Self::hash_u64(hash, vertex.x.to_bits());
        Self::hash_u64(hash, u64::from(vertex.height_m.to_bits()));
        Self::hash_u64(hash, vertex.z.to_bits());
    }

    fn hash_terrain_cdt_boundary_source(hash: &mut u64, source: TerrainCdtRoadBoundarySource) {
        match source {
            TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                edge_idx,
                edge_class,
                support_policy,
                source_band_index,
                band_kind,
                role,
                start_section_index,
                end_section_index,
                start_s_m,
                end_s_m,
            } => {
                Self::hash_u64(hash, 0);
                Self::hash_u64(hash, edge_idx);
                Self::hash_i64(
                    hash,
                    i64::from(
                        TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                            edge_idx,
                            edge_class,
                            support_policy,
                            source_band_index,
                            band_kind,
                            role,
                            start_section_index,
                            end_section_index,
                            start_s_m,
                            end_s_m,
                        }
                        .edge_class_code(),
                    ),
                );
                Self::hash_i64(
                    hash,
                    i64::from(
                        TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                            edge_idx,
                            edge_class,
                            support_policy,
                            source_band_index,
                            band_kind,
                            role,
                            start_section_index,
                            end_section_index,
                            start_s_m,
                            end_s_m,
                        }
                        .support_policy_code(),
                    ),
                );
                Self::hash_u64(hash, u64::from(source_band_index));
                Self::hash_i64(
                    hash,
                    i64::from(
                        TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                            edge_idx,
                            edge_class,
                            support_policy,
                            source_band_index,
                            band_kind,
                            role,
                            start_section_index,
                            end_section_index,
                            start_s_m,
                            end_s_m,
                        }
                        .owner_kind_code(),
                    ),
                );
                Self::hash_i64(
                    hash,
                    i64::from(
                        TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                            edge_idx,
                            edge_class,
                            support_policy,
                            source_band_index,
                            band_kind,
                            role,
                            start_section_index,
                            end_section_index,
                            start_s_m,
                            end_s_m,
                        }
                        .role_code(),
                    ),
                );
                Self::hash_u64(hash, u64::from(start_section_index));
                Self::hash_u64(hash, u64::from(end_section_index));
                Self::hash_u64(hash, u64::from(start_s_m.to_bits()));
                Self::hash_u64(hash, u64::from(end_s_m.to_bits()));
            }
            TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
                node_id,
                node_kind,
                owner_kind,
                owner_index,
                boundary_source,
            } => {
                let complete_source = TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
                    node_id,
                    node_kind,
                    owner_kind,
                    owner_index,
                    boundary_source,
                };
                Self::hash_u64(hash, 1);
                Self::hash_u64(hash, u64::from(node_id));
                Self::hash_i64(hash, i64::from(complete_source.node_kind_code()));
                Self::hash_i64(hash, i64::from(complete_source.owner_kind_code()));
                Self::hash_u64(hash, u64::from(owner_index));
                Self::hash_terrain_cdt_boundary_segment_source(hash, boundary_source);
            }
            TerrainCdtRoadBoundarySource::NodeSameMaterialBoundaryHandoff {
                node_id,
                node_kind,
                owner_kind,
                owner_index_a,
                owner_index_b,
                boundary_source,
            } => {
                let complete_source =
                    TerrainCdtRoadBoundarySource::NodeSameMaterialBoundaryHandoff {
                        node_id,
                        node_kind,
                        owner_kind,
                        owner_index_a,
                        owner_index_b,
                        boundary_source,
                    };
                Self::hash_u64(hash, 2);
                Self::hash_u64(hash, u64::from(node_id));
                Self::hash_i64(hash, i64::from(complete_source.node_kind_code()));
                Self::hash_i64(hash, i64::from(complete_source.owner_kind_code()));
                Self::hash_u64(hash, u64::from(owner_index_a));
                Self::hash_u64(hash, u64::from(owner_index_b));
                Self::hash_terrain_cdt_boundary_segment_source(hash, boundary_source);
            }
            TerrainCdtRoadBoundarySource::BuildingSiteBoundary {
                building_idx,
                local_loop_index,
                local_edge_index,
            } => {
                Self::hash_u64(hash, 3);
                Self::hash_u64(hash, building_idx);
                Self::hash_u64(hash, u64::from(local_loop_index));
                Self::hash_u64(hash, u64::from(local_edge_index));
            }
            TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                stable_piece_id,
                local_loop_index,
                local_edge_index,
            } => {
                Self::hash_u64(hash, 4);
                Self::hash_u64(hash, stable_piece_id);
                Self::hash_u64(hash, u64::from(local_loop_index));
                Self::hash_u64(hash, u64::from(local_edge_index));
            }
        }
    }

    fn hash_terrain_cdt_boundary_segment_source(
        hash: &mut u64,
        source: Option<
            crate::simulation::terrain::cdt::TerrainCdtNodeFootprintBoundarySegmentSource,
        >,
    ) {
        let Some(source) = source else {
            Self::hash_u64(hash, 0);
            return;
        };
        Self::hash_u64(hash, 1);
        Self::hash_terrain_cdt_boundary_vertex_source(hash, source.start);
        Self::hash_terrain_cdt_boundary_vertex_source(hash, source.end);
    }

    fn hash_terrain_cdt_boundary_vertex_source(
        hash: &mut u64,
        source: crate::simulation::terrain::cdt::TerrainCdtNodeFootprintBoundaryVertexSource,
    ) {
        use crate::simulation::terrain::cdt::TerrainCdtNodeFootprintBoundaryVertexSource;

        match source {
            TerrainCdtNodeFootprintBoundaryVertexSource::Direct(source) => {
                Self::hash_u64(hash, 0);
                Self::hash_u64(hash, source.top_surface_source_index);
                Self::hash_u64(hash, source.grade_authority_index);
            }
            TerrainCdtNodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                x_key,
                z_key,
                y_mm,
            } => {
                Self::hash_u64(hash, 1);
                Self::hash_i64(hash, x_key);
                Self::hash_i64(hash, z_key);
                Self::hash_i64(hash, y_mm);
            }
            TerrainCdtNodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                owning_segment_start,
                owning_segment_end,
                height_mm,
            } => {
                Self::hash_u64(hash, 2);
                Self::hash_u64(hash, owning_segment_start.top_surface_source_index);
                Self::hash_u64(hash, owning_segment_start.grade_authority_index);
                Self::hash_u64(hash, owning_segment_end.top_surface_source_index);
                Self::hash_u64(hash, owning_segment_end.grade_authority_index);
                Self::hash_i64(hash, height_mm);
            }
        }
    }

    pub(in crate::nodes::simulation_node) fn hash_i64(hash: &mut u64, value: i64) {
        Self::hash_u64(hash, value as u64);
    }

    pub(in crate::nodes::simulation_node) fn hash_u64(hash: &mut u64, value: u64) {
        *hash ^= value;
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_patch_for_bounds(
        terrain: &TerrainSystem,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> TerrainCdtPatch {
        TerrainCdtPatch::new(
            f64::from(min_x),
            f64::from(min_z),
            f64::from(max_x),
            f64::from(max_z),
            [
                terrain.sample_visual_height_world(min_x, min_z) * config::HEIGHT_SCALE,
                terrain.sample_visual_height_world(min_x, max_z) * config::HEIGHT_SCALE,
                terrain.sample_visual_height_world(max_x, max_z) * config::HEIGHT_SCALE,
                terrain.sample_visual_height_world(max_x, min_z) * config::HEIGHT_SCALE,
            ],
        )
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_local_sample_bounds(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        road_loops: &[TerrainCdtRoadLoop],
        render_step_m: f32,
    ) -> Option<(f32, f32, f32, f32)> {
        Self::terrain_cdt_local_sample_bounds_for_world_bounds(
            terrain,
            (
                patch.world_origin_x,
                patch.world_origin_z,
                patch.world_origin_x + patch.world_size_x,
                patch.world_origin_z + patch.world_size_z,
            ),
            road_loops,
            render_step_m,
        )
    }

    /// Returns the exact road-grading influence clipped to fixed world bounds.
    pub(in crate::nodes::simulation_node) fn terrain_cdt_local_sample_bounds_for_world_bounds(
        terrain: &TerrainSystem,
        patch_bounds: (f32, f32, f32, f32),
        road_loops: &[TerrainCdtRoadLoop],
        render_step_m: f32,
    ) -> Option<(f32, f32, f32, f32)> {
        let mut min_x = f32::INFINITY;
        let mut min_z = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_z = f32::NEG_INFINITY;
        for road_loop in road_loops {
            for vertex in &road_loop.vertices {
                let x = vertex.x as f32;
                let z = vertex.z as f32;
                min_x = min_x.min(x);
                min_z = min_z.min(z);
                max_x = max_x.max(x);
                max_z = max_z.max(z);
            }
        }
        if !min_x.is_finite() || !min_z.is_finite() || !max_x.is_finite() || !max_z.is_finite() {
            return None;
        }

        let margin_m = RoadSurfaceSystem::terrain_cdt_required_grading_margin_m(
            terrain,
            road_loops,
            render_step_m,
        );
        let (patch_min_x, patch_min_z, patch_max_x, patch_max_z) = patch_bounds;
        min_x = (min_x - margin_m).clamp(patch_min_x, patch_max_x);
        min_z = (min_z - margin_m).clamp(patch_min_z, patch_max_z);
        max_x = (max_x + margin_m).clamp(patch_min_x, patch_max_x);
        max_z = (max_z + margin_m).clamp(patch_min_z, patch_max_z);
        if max_x <= min_x + TERRAIN_CDT_MIN_WINDOW_EXTENT_M
            || max_z <= min_z + TERRAIN_CDT_MIN_WINDOW_EXTENT_M
        {
            None
        } else {
            Some((min_x, min_z, max_x, max_z))
        }
    }

    pub(in crate::nodes::simulation_node) fn append_terrain_cdt_grid_samples(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        step_m: f32,
        source_samples: &mut Vec<TerrainCdtVertex>,
        sample_keys: &mut HashSet<(i64, i64)>,
    ) {
        let safe_step_m = step_m.max(f32::EPSILON);
        let patch_min_x = patch.world_origin_x;
        let patch_min_z = patch.world_origin_z;
        let patch_max_x = patch.world_origin_x + patch.world_size_x;
        let patch_max_z = patch.world_origin_z + patch.world_size_z;
        let start_x_index = (((min_x.clamp(patch_min_x, patch_max_x) - patch_min_x) / safe_step_m)
            .floor() as i64)
            .max(0);
        let start_z_index = (((min_z.clamp(patch_min_z, patch_max_z) - patch_min_z) / safe_step_m)
            .floor() as i64)
            .max(0);
        let end_x_index = (((max_x.clamp(patch_min_x, patch_max_x) - patch_min_x) / safe_step_m)
            .ceil() as i64)
            .max(start_x_index);
        let end_z_index = (((max_z.clamp(patch_min_z, patch_max_z) - patch_min_z) / safe_step_m)
            .ceil() as i64)
            .max(start_z_index);

        for sample_z_index in start_z_index..=end_z_index {
            let world_z = (patch_min_z + sample_z_index as f32 * safe_step_m).min(patch_max_z);
            for sample_x_index in start_x_index..=end_x_index {
                let world_x = (patch_min_x + sample_x_index as f32 * safe_step_m).min(patch_max_x);
                Self::push_terrain_cdt_source_sample(
                    terrain,
                    world_x,
                    world_z,
                    source_samples,
                    sample_keys,
                );
            }
        }
    }

    pub(in crate::nodes::simulation_node) fn append_terrain_cdt_window_boundary_samples(
        terrain: &TerrainSystem,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        step_m: f32,
        source_samples: &mut Vec<TerrainCdtVertex>,
        sample_keys: &mut HashSet<(i64, i64)>,
    ) {
        let safe_step_m = step_m.max(f32::EPSILON);
        let xs = Self::terrain_cdt_axis_samples(min_x, max_x, safe_step_m);
        let zs = Self::terrain_cdt_axis_samples(min_z, max_z, safe_step_m);
        for &x in &xs {
            Self::push_terrain_cdt_source_sample(terrain, x, min_z, source_samples, sample_keys);
            Self::push_terrain_cdt_source_sample(terrain, x, max_z, source_samples, sample_keys);
        }
        for &z in &zs {
            Self::push_terrain_cdt_source_sample(terrain, min_x, z, source_samples, sample_keys);
            Self::push_terrain_cdt_source_sample(terrain, max_x, z, source_samples, sample_keys);
        }
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_axis_samples(
        min: f32,
        max: f32,
        step_m: f32,
    ) -> Vec<f32> {
        let step_mm = Self::quantize_cdt_coord_mm(f64::from(step_m.max(0.001))).max(1);
        let min_mm = Self::quantize_cdt_coord_mm(f64::from(min));
        let max_mm = Self::quantize_cdt_coord_mm(f64::from(max));
        let mut samples = vec![min];
        let mut lattice_index = min_mm.div_euclid(step_mm) + 1;
        loop {
            let next_mm = lattice_index.saturating_mul(step_mm);
            if next_mm >= max_mm {
                break;
            }
            samples.push(next_mm as f32 / 1000.0);
            lattice_index += 1;
        }
        if max_mm > min_mm {
            samples.push(max);
        }
        samples
    }

    pub(in crate::nodes::simulation_node) fn push_terrain_cdt_source_sample(
        terrain: &TerrainSystem,
        world_x: f32,
        world_z: f32,
        source_samples: &mut Vec<TerrainCdtVertex>,
        sample_keys: &mut HashSet<(i64, i64)>,
    ) {
        let key = (
            round_f64_to_i64(f64::from(world_x) * TERRAIN_CDT_SAMPLE_KEY_SCALE),
            round_f64_to_i64(f64::from(world_z) * TERRAIN_CDT_SAMPLE_KEY_SCALE),
        );
        if !sample_keys.insert(key) {
            return;
        }
        source_samples.push(TerrainCdtVertex::new(
            f64::from(world_x),
            terrain.sample_visual_height_world(world_x, world_z) * config::HEIGHT_SCALE,
            f64::from(world_z),
        ));
    }

    pub(in crate::nodes::simulation_node) fn terrain_patch_sample_height_m(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        sample_x: usize,
        sample_z: usize,
    ) -> f32 {
        if patch.texture_width == 0 || patch.height_data.is_empty() {
            return 0.0;
        }
        let texture_x = patch
            .inner_offset_x
            .saturating_add(sample_x.min(patch.sample_width.saturating_sub(1)));
        let texture_z = patch
            .inner_offset_z
            .saturating_add(sample_z.min(patch.sample_height.saturating_sub(1)));
        let index = texture_z
            .saturating_mul(patch.texture_width)
            .saturating_add(texture_x)
            .min(patch.height_data.len().saturating_sub(1));
        patch.height_data[index] * config::HEIGHT_SCALE
    }

    pub(in crate::nodes::simulation_node) fn terrain_patch_height_at_world_m(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        world_x: f32,
        world_z: f32,
    ) -> f32 {
        if patch.sample_width == 0 || patch.sample_height == 0 {
            return 0.0;
        }
        let local_x = ((world_x - patch.world_origin_x) / patch.world_size_x.max(0.001))
            .clamp(0.0, 1.0)
            * patch.sample_width.saturating_sub(1) as f32;
        let local_z = ((world_z - patch.world_origin_z) / patch.world_size_z.max(0.001))
            .clamp(0.0, 1.0)
            * patch.sample_height.saturating_sub(1) as f32;

        let x0 = local_x.floor() as usize;
        let z0 = local_z.floor() as usize;
        let x1 = (x0 + 1).min(patch.sample_width.saturating_sub(1));
        let z1 = (z0 + 1).min(patch.sample_height.saturating_sub(1));
        let tx = local_x.fract();
        let tz = local_z.fract();

        let h00 = Self::terrain_patch_sample_height_m(patch, x0, z0);
        let h10 = Self::terrain_patch_sample_height_m(patch, x1, z0);
        let h01 = Self::terrain_patch_sample_height_m(patch, x0, z1);
        let h11 = Self::terrain_patch_sample_height_m(patch, x1, z1);
        let h0 = h00 * (1.0 - tx) + h10 * tx;
        let h1 = h01 * (1.0 - tx) + h11 * tx;
        h0 * (1.0 - tz) + h1 * tz
    }
}
