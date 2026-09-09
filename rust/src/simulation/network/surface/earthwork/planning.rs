// SPDX-License-Identifier: GPL-2.0-only

//! Bounded planned road terrain cutout inputs and exact reusable structural stamp products.

use super::super::{RoadSurfaceTerrainClipExportError, RoadSurfaceTerrainClipLoop};
use super::stamping::{
    EarthworkChunkStampBuilder, EarthworkChunkStampResult, EarthworkStampTriangle,
};
use super::*;
use crate::simulation::terrain::cdt::TerrainCdtRoadLoop;
use crate::simulation::terrain::{TerrainPatchSnapshot, TerrainVisualOverlay};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Local road-only clipping input; building contributors must still be checked at commit.
#[derive(Debug)]
pub(crate) struct PlannedRoadTerrainClipPatch {
    /// Unmodified visual patch used to prepare patch-local terrain buffers.
    pub(crate) patch: Arc<TerrainPatchSnapshot>,
    /// Exact final road grading ownership, independent of padded clip-query hits.
    pub(crate) road_owned: bool,
    /// Production clip query margin for this patch's actual road/site grading ownership.
    pub(crate) query_margin_m: f32,
    /// Final road cutout loops with prospective live owner identities, or an export failure.
    pub(crate) loops: Result<(Vec<TerrainCdtRoadLoop>, usize), RoadSurfaceTerrainClipExportError>,
}

#[derive(Clone, Debug, PartialEq)]
struct EarthworkGridDependency {
    source_generation: u64,
    dimensions: (usize, usize),
    cell_size: u32,
    origin: (u32, u32),
    storage_chunk_size: usize,
    surface_grid: [u32; 3],
}

impl EarthworkGridDependency {
    fn capture(surface: &RoadSurfaceSystem, terrain: &TerrainSystem) -> Self {
        let (x, z) = terrain.grid_to_world_coords(0, 0);
        Self {
            source_generation: terrain.source_generation(),
            dimensions: terrain.grid_dimensions(),
            cell_size: terrain.cell_size_m().to_bits(),
            origin: (x.to_bits(), z.to_bits()),
            storage_chunk_size: terrain.storage_chunk_size_cells(),
            surface_grid: [
                surface.chunk_span_m.to_bits(),
                surface.chunk_origin_x_m.to_bits(),
                surface.chunk_origin_z_m.to_bits(),
            ],
        }
    }
}

#[derive(Debug)]
struct PlannedEarthworkChunk {
    triangles: Vec<EarthworkStampTriangle>,
    result: EarthworkChunkStampResult,
}

/// Read-only products of the existing stamper for the old/new local road ownership footprint.
/// Retains structural visual writes and road cutout inputs; the renderer's CDT compiler owns tiles.
#[derive(Debug)]
pub(crate) struct RoadEarthworkPlan {
    grid: EarthworkGridDependency,
    chunks: HashMap<SurfaceChunkKey, PlannedEarthworkChunk>,
    /// Final structural resets and writes, in the same sorted chunk order as publication.
    pub(crate) visual: TerrainVisualOverlay,
    /// Read-only inputs for the existing terrain CDT compiler.
    pub(crate) clip_patches: Vec<PlannedRoadTerrainClipPatch>,
    /// Final local road queries used by building-site grading in the preview worker.
    pub(crate) roads: super::super::PlannedRoadSurfaceQuery,
}

impl RoadEarthworkPlan {
    /// Captures the reset footprint, including unchanged chunks that a rollback must preserve.
    pub(crate) fn visual_checkpoint(&self, terrain: &TerrainSystem) -> TerrainVisualOverlay {
        let mut checkpoint = TerrainVisualOverlay::new(terrain);
        let span = f64::from(f32::from_bits(self.grid.surface_grid[0]));
        let ox = f64::from(f32::from_bits(self.grid.surface_grid[1]));
        let oz = f64::from(f32::from_bits(self.grid.surface_grid[2]));
        for key in self.chunks.keys() {
            let x = ox + f64::from(key.0) * span;
            let z = oz + f64::from(key.1) * span;
            checkpoint.capture_region(
                terrain,
                x as f32,
                z as f32,
                (x + span) as f32,
                (z + span) as f32,
            );
        }
        checkpoint
    }

    /// Whether structural stamping would write visual samples after this candidate was compiled.
    #[cfg(test)]
    pub(crate) fn has_visual_stamp_writes(&self) -> bool {
        self.chunks
            .values()
            .any(|chunk| !chunk.result.writes.is_empty())
    }

    pub(super) fn dependencies_match(
        &self,
        surface: &RoadSurfaceSystem,
        terrain: &TerrainSystem,
    ) -> bool {
        self.grid == EarthworkGridDependency::capture(surface, terrain)
    }

    pub(super) fn matching_chunk(
        &self,
        chunk: SurfaceChunkKey,
        input: &EarthworkChunkStampBuilder<'_>,
    ) -> Option<&EarthworkChunkStampResult> {
        let planned = self.chunks.get(&chunk)?;
        // Compare actual ordered inputs, not just a hash, IDs or a coarse revision. Changed
        // support geometry, offsets, visibility and neighbouring contributors all miss safely.
        (planned.triangles == input.triangles).then_some(&planned.result)
    }
}

impl RoadSurfaceSystem {
    /// Compiles only the replacement owners' old/new chunks, retaining all other contributors
    /// through the existing earthwork chunk index. IDs must match topology adoption order.
    /// Work is O(local coverage + sum(chunk owners * log(chunk owners)) + stamp raster work).
    pub(crate) fn plan_earthwork_overlay(
        &self,
        graph: &RegionGraph,
        existing: &Self,
        existing_graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_ids: &[usize],
        node_ids: &[u32],
        replaced_edges: &HashSet<usize>,
        replaced_nodes: &HashSet<u32>,
    ) -> Option<RoadEarthworkPlan> {
        let grid = EarthworkGridDependency::capture(self, terrain);
        if grid != EarthworkGridDependency::capture(existing, terrain) {
            return None;
        }
        let mut chunks = BTreeSet::new();
        for id in replaced_edges {
            if let Some(coverage) = existing.earthwork_span_chunks.get(id) {
                chunks.extend(coverage.iter().copied());
            }
        }
        for id in replaced_nodes {
            if let Some(coverage) = existing.earthwork_node_chunks.get(id) {
                chunks.extend(coverage.iter().copied());
            }
        }
        // `self` is the bounded validation excerpt, never the resident city's surface cache.
        chunks.extend(self.earthwork_chunk_cache.keys().copied());
        let chunks = chunks.into_iter().collect::<Vec<_>>();
        // Same bounded dirty-patch envelope as commit. The conservative query discovers road
        // contributors; exact tile influence and input checks decide what can be reused.
        let render_step_m = crate::simulation::terrain::ROAD_LOCKED_TERRAIN_RENDER_STEP_M;
        let patch_keys =
            self.render_patch_keys_for_chunk_grading_envelopes(terrain, &chunks, render_step_m);
        let planned_chunks: HashMap<_, _> = chunks
            .par_iter()
            .map(|&chunk| {
                let mut edges = BTreeMap::new();
                let mut nodes = BTreeMap::new();
                if let Some(entry) = existing.earthwork_chunk_cache.get(&chunk) {
                    edges.extend(entry.edge_indices.iter().filter_map(|&id| {
                        (!replaced_edges.contains(&id)).then_some((id, (existing, id)))
                    }));
                    nodes.extend(entry.node_ids.iter().filter_map(|&id| {
                        (!replaced_nodes.contains(&id))
                            .then_some((id, (existing, existing_graph, id)))
                    }));
                }
                if let Some(entry) = self.earthwork_chunk_cache.get(&chunk) {
                    edges.extend(
                        entry
                            .edge_indices
                            .iter()
                            .map(|&id| (edge_ids[id], (self, id))),
                    );
                    nodes.extend(
                        entry
                            .node_ids
                            .iter()
                            .map(|&id| (node_ids[id as usize], (self, graph, id))),
                    );
                }
                let mut builder = EarthworkChunkStampBuilder::new(self, terrain, chunk);
                if !edges.is_empty() || !nodes.is_empty() {
                    builder.mark_cache_present();
                }
                // Live caches are ordered by owner ID: preserve exactly that traversal even
                // when preview-local slots differ, including owners outside the graph excerpt.
                for (_, (surface, id)) in edges {
                    surface.collect_span_stamp_input(id, &mut builder);
                }
                for (_, (surface, owner_graph, id)) in nodes {
                    surface.collect_node_stamp_input(owner_graph, terrain, id, &mut builder);
                }
                let triangles = builder.triangles.clone();
                let result = builder.finish();
                (chunk, PlannedEarthworkChunk { triangles, result })
            })
            .collect();
        let mut visual = TerrainVisualOverlay::new(terrain);
        // Resets and writes must stay interleaved: neighboring chunks share inclusive border
        // samples, and a later reset can replace an earlier stamp at that border.
        for chunk in chunks {
            let (min, max) = self.chunk_bounds(chunk);
            visual.reset_region_from_source_world(
                terrain,
                min.x as f32,
                min.z as f32,
                max.x as f32,
                max.z as f32,
            );
            visual.set_heights(terrain, &planned_chunks[&chunk].result.writes, |write| {
                (write.grid_x, write.grid_z, write.height_sample)
            });
        }
        visual.discard_unchanged(terrain);
        // Discover ownership and clip queries against the final stamped samples.
        let final_visual = visual.view(terrain);
        // Ownership is solved independently of CDT success, exactly as for fresh live patches.
        // Exclude superseded owners before combining grading; their old footprint only dirties
        // patches and must not keep a vacated patch road-locked. Both queries stay local.
        let mut road_margins = existing.terrain_render_patch_grading_margins_for_filtered_patches(
            existing_graph,
            &final_visual,
            render_step_m,
            &patch_keys,
            |id| !replaced_edges.contains(&id),
            |id| !replaced_nodes.contains(&id),
        );
        for (key, planned_margin) in self.terrain_render_patch_grading_margins_for_patches(
            graph,
            &final_visual,
            render_step_m,
            &patch_keys,
        ) {
            road_margins
                .entry(key)
                .and_modify(|margin| *margin = margin.max(planned_margin))
                .or_insert(planned_margin);
        }
        let site_margin =
            crate::simulation::terrain::terrain_cdt_local_sample_margin_m(terrain, render_step_m);
        let clip_patches = patch_keys
            .into_par_iter()
            .filter_map(|(x, z)| {
                let patch = terrain.visual_patch_snapshot(x, z)?;
                let road_margin = road_margins.get(&(x, z));
                let query_margin_m = road_margin.map_or(site_margin, |&margin| {
                    crate::simulation::terrain::terrain_cdt_road_query_margin_m(
                        terrain, render_step_m, margin.max(site_margin),
                    )
                });
                let bounds = (patch.world_origin_x - query_margin_m, patch.world_origin_z - query_margin_m,
                    patch.world_origin_x + patch.world_size_x + query_margin_m,
                    patch.world_origin_z + patch.world_size_z + query_margin_m);
                let mut old = existing.terrain_clip_boundary_loops_for_world_bounds(
                    existing_graph, bounds.0, bounds.1, bounds.2, bounds.3,
                );
                old.retain(|boundary| boundary.source_edges.iter().all(|edge| match edge.source {
                    RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { edge_idx, .. } => !replaced_edges.contains(&edge_idx),
                    RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary { node_id, .. }
                    | RoadSurfaceEarthworkFaceSource::NodeSameMaterialBoundaryHandoff { node_id, .. } => !replaced_nodes.contains(&node_id),
                }));
                let mapped = self.terrain_clip_boundary_loops_for_world_bounds(
                    graph, bounds.0, bounds.1, bounds.2, bounds.3,
                ).into_iter().map(|boundary| {
                    let mut boundary = boundary.clone();
                    for edge in &mut boundary.source_edges {
                        edge.source = match edge.source {
                            RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { edge_idx, .. } => edge.source.with_span_identity(edge_ids[edge_idx]),
                            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary { node_id, kind, .. }
                            | RoadSurfaceEarthworkFaceSource::NodeSameMaterialBoundaryHandoff { node_id, kind, .. } => edge.source.with_node_identity(node_ids[node_id as usize], kind),
                        };
                    }
                    boundary
                }).collect::<Vec<_>>();
                old.extend(mapped.iter());
                // Union only after removing superseded owners, and in live owner order.
                old.sort_by_key(|boundary| clip_owner_key(boundary));
                let loops = Self::terrain_cdt_road_loops_from_boundaries(&old);
                Some(PlannedRoadTerrainClipPatch {
                    patch: Arc::new(patch), road_owned: road_margin.is_some(), query_margin_m, loops,
                })
            }).collect();
        drop(final_visual);
        Some(RoadEarthworkPlan {
            grid,
            chunks: planned_chunks,
            visual,
            clip_patches,
            roads: super::super::PlannedRoadSurfaceQuery::capture(
                graph,
                self,
                edge_ids,
                node_ids,
                replaced_edges,
                replaced_nodes,
            ),
        })
    }
}

fn clip_owner_key(boundary: &RoadSurfaceTerrainClipLoop) -> (u8, usize) {
    match boundary.source_edges.first().map(|edge| edge.source) {
        Some(RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { edge_idx, .. }) => (0, edge_idx),
        Some(RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary { node_id, .. })
        | Some(RoadSurfaceEarthworkFaceSource::NodeSameMaterialBoundaryHandoff {
            node_id, ..
        }) => (1, node_id as usize),
        None => (2, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::{
        build_surface_edge,
        types::{EdgeClass, NodeType},
    };
    use godot::prelude::Vector3;

    #[test]
    fn replaced_road_ownership_does_not_lock_vacated_patches() {
        let terrain = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
        let make_road = |z| {
            let mut graph = RegionGraph::new();
            let a = graph.add_node(Vector3::new(-48.0, 0.0, z), NodeType::Junction);
            let b = graph.add_node(Vector3::new(48.0, 0.0, z), NodeType::Junction);
            graph.add_edge(build_surface_edge(
                a,
                b,
                vec![graph.node(a).pos, graph.node(b).pos],
                1,
                1,
                EdgeClass::Standard,
            ));
            let mut surface = RoadSurfaceSystem::new(64.0);
            assert!(surface.compile_dirty(&graph, &terrain));
            (graph, surface)
        };
        let (old_graph, old) = make_road(-20.0);
        let (graph, surface) = make_road(20.0);
        let plan = surface
            .plan_earthwork_overlay(
                &graph,
                &old,
                &old_graph,
                &terrain,
                &[1],
                &[2, 3],
                &HashSet::from([0]),
                &HashSet::from([0, 1]),
            )
            .unwrap();
        let keys = plan
            .clip_patches
            .iter()
            .map(|clip| (clip.patch.patch_x, clip.patch.patch_z))
            .collect::<Vec<_>>();
        let actual =
            surface.terrain_render_patch_grading_margins_for_patches(&graph, &terrain, 2.0, &keys);
        assert!(keys.contains(&(0, 0)) && keys.contains(&(0, 1)));
        assert!(!actual.contains_key(&(0, 0)));
        assert!(actual.contains_key(&(0, 1)));
        for clip in &plan.clip_patches {
            let key = (clip.patch.patch_x, clip.patch.patch_z);
            assert_eq!(clip.road_owned, actual.contains_key(&key), "{key:?}");
        }
    }

    #[test]
    fn planned_visible_tunnel_portal_reuses_nonempty_stamps() {
        let mut terrain = TerrainSystem::with_chunking(97, 97, 1.0, 8, 0.0);
        let mut graph = RegionGraph::new();
        let a = graph.add_node(Vector3::new(-24.0, -0.5, 0.0), NodeType::Junction);
        let b = graph.add_node(Vector3::new(24.0, -0.5, 0.0), NodeType::Junction);
        graph.add_edge(build_surface_edge(
            a,
            b,
            vec![graph.node(a).pos, graph.node(b).pos],
            1,
            1,
            EdgeClass::Tunnel,
        ));
        let mut surface = RoadSurfaceSystem::new(16.0);
        assert!(surface.compile_dirty(&graph, &terrain));
        let empty_surface = RoadSurfaceSystem::new(16.0);
        let plan = surface
            .plan_earthwork_overlay(
                &graph,
                &empty_surface,
                &RegionGraph::new(),
                &terrain,
                &[0],
                &[0, 1],
                &HashSet::new(),
                &HashSet::new(),
            )
            .unwrap();
        assert!(
            plan.chunks
                .values()
                .any(|chunk| !chunk.result.writes.is_empty())
        );
        assert!(plan.has_visual_stamp_writes());
        for (&key, chunk) in &plan.chunks {
            let cold = surface
                .prepare_earthwork_chunk_stamp(&graph, &terrain, key)
                .finish();
            assert_eq!(chunk.result.writes, cold.writes);
        }
        let plan = Arc::new(plan);
        let before = terrain.clone(); // Independent oracle for the pinned read-only input.
        let keys = plan
            .clip_patches
            .iter()
            .map(|clip| (clip.patch.patch_x, clip.patch.patch_z))
            .collect::<Vec<_>>();
        // Seed the resident pre-stamp cache; publication must reject it after visual writes.
        surface.terrain_render_patch_grading_margins_for_patches(&graph, &terrain, 2.0, &keys);
        surface.enqueue_planned_earthworks(Some(Arc::clone(&plan)));
        let chunks = surface.rebuild_dirty_earthworks(&graph, &mut terrain);
        assert_eq!(surface.last_reused_earthwork_chunk_count, chunks.len());
        assert!(
            terrain.sample_visual_height_world(-20.0, 0.0)
                < terrain.sample_height_world(-20.0, 0.0)
        );
        use crate::simulation::terrain::TerrainVisualSource;
        let visual = plan.visual.view(&before);
        for z in 0..terrain.height {
            for x in 0..terrain.width {
                let (wx, wz) = terrain.grid_to_world_coords(x, z);
                assert_eq!(
                    visual.sample_visual_height_world(wx, wz),
                    terrain.sample_visual_height_world(wx, wz),
                    "({x}, {z})"
                );
                assert_eq!(before.get_height(x, z), terrain.get_height(x, z));
            }
        }
        let margins =
            surface.terrain_render_patch_grading_margins_for_patches(&graph, &terrain, 2.0, &keys);
        for clip in &plan.clip_patches {
            let key = (clip.patch.patch_x, clip.patch.patch_z);
            assert_eq!(clip.road_owned, margins.contains_key(&key));
            let site_margin =
                crate::simulation::terrain::terrain_cdt_local_sample_margin_m(&terrain, 2.0);
            let margin = margins.get(&key).map_or(site_margin, |&margin| {
                crate::simulation::terrain::terrain_cdt_road_query_margin_m(
                    &terrain,
                    2.0,
                    margin.max(site_margin),
                )
            });
            assert_eq!(clip.query_margin_m, margin);
            let patch = &clip.patch;
            let boundaries = surface.terrain_clip_boundary_loops_for_world_bounds(
                &graph,
                patch.world_origin_x - margin,
                patch.world_origin_z - margin,
                patch.world_origin_x + patch.world_size_x + margin,
                patch.world_origin_z + patch.world_size_z + margin,
            );
            let loops =
                RoadSurfaceSystem::terrain_cdt_road_loops_from_boundaries(&boundaries).unwrap();
            assert_eq!(
                clip.loops.as_ref().unwrap(),
                &loops,
                "post-stamp cutouts {key:?}"
            );
            assert_eq!(
                plan.visual.patch_snapshot(&before, &clip.patch).as_ref(),
                &terrain
                    .visual_patch_snapshot(clip.patch.patch_x, clip.patch.patch_z)
                    .unwrap()
            );
        }
    }
}
