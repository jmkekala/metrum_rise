// SPDX-License-Identifier: GPL-2.0-only

//! Shared, bounded site feasibility for demand and non-mutating zoning checks.

mod zoning;

use super::*;
use crate::simulation::buildings::allocator::BuildingSiteTerrainSnapshot;
use crate::simulation::network::surface::{RoadSurfaceVisualNodePiece, RoadSurfaceVisualSpanPiece};
use crate::simulation::terrain::TerrainVisualOverlay;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Live geometry inputs for the shared roadside-site solver.
#[derive(Clone, Copy)]
pub(crate) struct BuildingSiteEnvironment<'a> {
    /// Compiled road surfaces from the same graph revision used for placement.
    pub(crate) road_surface: &'a RoadSurfaceSystem,
    /// Current source and engineered visual terrain.
    pub(crate) terrain: &'a TerrainSystem,
}

type Bounds = (f32, f32, f32, f32);
type Epoch = (u64, u64, u64, u64, u64);
type Verdict = Result<f32, DemandSpawnPlacementRejection>;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PoseKey {
    replaced_building: Option<usize>,
    parcel: u64,
    edge: usize,
    side: i8,
    profile: u16,
    width: usize,
    depth: usize,
    bits: [u32; 6],
}

impl PoseKey {
    fn new(p: &ResolvedPlacement) -> Self {
        Self {
            replaced_building: p.replaced_building,
            parcel: p.parcel_id,
            edge: p.edge_idx,
            side: p.side,
            profile: p.zone_profile_runtime_id,
            width: p.width_cells,
            depth: p.depth_cells,
            bits: [
                p.center_2d.x.to_bits(),
                p.center_2d.y.to_bits(),
                p.facing_dir.x.to_bits(),
                p.facing_dir.y.to_bits(),
                p.frontage_t.to_bits(),
                p.zone_cell_m.to_bits(),
            ],
        }
    }
}

struct Dependencies {
    bounds: Bounds,
    terrain: TerrainVisualOverlay,
    spans: BTreeMap<usize, Arc<RoadSurfaceVisualSpanPiece>>,
    nodes: BTreeMap<u32, Arc<RoadSurfaceVisualNodePiece>>,
    sites: BuildingSiteTerrainSnapshot,
}

impl Dependencies {
    fn roads(
        bounds: Bounds,
        surface: &RoadSurfaceSystem,
    ) -> (
        BTreeMap<usize, Arc<RoadSurfaceVisualSpanPiece>>,
        BTreeMap<u32, Arc<RoadSurfaceVisualNodePiece>>,
    ) {
        let mut spans = BTreeMap::new();
        let mut nodes = BTreeMap::new();
        let min = RoadSurfaceSystem::query_chunk_coords_for_world(
            f64::from(bounds.0),
            f64::from(bounds.1),
        );
        let max = RoadSurfaceSystem::query_chunk_coords_for_world(
            f64::from(bounds.2),
            f64::from(bounds.3),
        );
        for cx in min.0..=max.0 {
            for cz in min.1..=max.1 {
                if let Some(ids) = surface.query_chunk_spans.get(&(cx, cz)) {
                    for id in ids {
                        if let Some(piece) = surface.compiled_visual_span_pieces.get(id) {
                            spans.insert(*id, Arc::clone(piece));
                        }
                    }
                }
                if let Some(ids) = surface.query_chunk_nodes.get(&(cx, cz)) {
                    for id in ids {
                        if let Some(piece) = surface.compiled_visual_node_pieces.get(id) {
                            nodes.insert(*id, Arc::clone(piece));
                        }
                    }
                }
            }
        }
        (spans, nodes)
    }

    fn capture(
        allocator: &BuildingAllocator,
        p: &ResolvedPlacement,
        env: BuildingSiteEnvironment<'_>,
    ) -> Self {
        let b = allocator.placement_site_bounds(p);
        Self::capture_bounds(allocator, b, env)
    }

    fn capture_bounds(
        allocator: &BuildingAllocator,
        b: Bounds,
        env: BuildingSiteEnvironment<'_>,
    ) -> Self {
        let margin = super::super::site::site_feasibility_dependency_margin_m(env.terrain);
        let bounds = (b.0 - margin, b.1 - margin, b.2 + margin, b.3 + margin);
        let mut terrain = TerrainVisualOverlay::new(env.terrain);
        terrain.capture_region(env.terrain, bounds.0, bounds.1, bounds.2, bounds.3);
        let (spans, nodes) = Self::roads(bounds, env.road_surface);
        let sites = allocator
            .terrain_site_snapshot_for_world_bounds(bounds.0, bounds.1, bounds.2, bounds.3);
        Self {
            bounds,
            terrain,
            spans,
            nodes,
            sites,
        }
    }

    fn matches(&self, allocator: &BuildingAllocator, env: BuildingSiteEnvironment<'_>) -> bool {
        if !self.terrain.matches_current(env.terrain) {
            return false;
        }
        let (spans, nodes) = Self::roads(self.bounds, env.road_surface);
        if spans.len() != self.spans.len()
            || nodes.len() != self.nodes.len()
            || !spans
                .iter()
                .all(|(id, p)| self.spans.get(id).is_some_and(|old| Arc::ptr_eq(old, p)))
            || !nodes
                .iter()
                .all(|(id, p)| self.nodes.get(id).is_some_and(|old| Arc::ptr_eq(old, p)))
        {
            return false;
        }
        let b = self.bounds;
        self.sites == allocator.terrain_site_snapshot_for_world_bounds(b.0, b.1, b.2, b.3)
    }
}

#[derive(Clone)]
struct CachedSite {
    epoch: Epoch,
    dependencies: Arc<Dependencies>,
    verdict: Verdict,
}

impl CachedSite {
    fn is_current(
        &self,
        epoch: Epoch,
        allocator: &BuildingAllocator,
        env: BuildingSiteEnvironment<'_>,
    ) -> bool {
        self.epoch.4 == epoch.4
            && (self.epoch == epoch || self.dependencies.matches(allocator, env))
    }
}

/// Derived cache only: cloning an allocator starts empty so independent edits cannot share epochs.
#[derive(Default)]
pub(crate) struct SiteFeasibilityCache {
    entries: Mutex<HashMap<String, HashMap<PoseKey, CachedSite>>>,
    previews: Mutex<HashMap<String, HashMap<PoseKey, CachedSite>>>,
    zoning: zoning::ZoningSiteCache,
    #[cfg(test)]
    solves: std::sync::atomic::AtomicU64,
}

impl SiteFeasibilityCache {
    fn poses(&self, key: PoseKey) -> &Mutex<HashMap<String, HashMap<PoseKey, CachedSite>>> {
        if key.parcel == 0 {
            &self.previews
        } else {
            &self.entries
        }
    }
}

impl Clone for SiteFeasibilityCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl BuildingAllocator {
    pub(super) fn prepare_site_support_cached(
        &self,
        p: &mut ResolvedPlacement,
        graph: &RegionGraph,
        env: BuildingSiteEnvironment<'_>,
    ) -> Result<(), DemandSpawnPlacementRejection> {
        // Failed or pending compilation must not authorize placement from retained old surfaces.
        if !env.road_surface.published_generation_matches_source() {
            return Err(DemandSpawnPlacementRejection::FrontageRoadSurfaceMissing);
        }
        let epoch = (
            env.terrain.source_generation(),
            env.terrain.visual_generation(),
            env.road_surface.compile_invalidation_generation,
            self.building_ref_revision(),
            self.registry.revision(),
        );
        let key = PoseKey::new(p);
        let previous = {
            let cache = self.site_feasibility.poses(key).lock().unwrap();
            cache
                .get(&p.asset_id)
                .and_then(|poses| poses.get(&key))
                .cloned()
        };
        let verdict =
            if let Some(previous) = previous.filter(|old| old.is_current(epoch, self, env)) {
                if previous.epoch != epoch
                    && let Some(old) = self
                        .site_feasibility
                        .poses(key)
                        .lock()
                        .unwrap()
                        .get_mut(&p.asset_id)
                        .and_then(|poses| poses.get_mut(&key))
                {
                    old.epoch = epoch;
                }
                previous.verdict
            } else {
                self.cache_site_solution(p, graph, env, key, epoch)
            };
        p.support_height_m = verdict?;
        Ok(())
    }

    fn cache_site_solution(
        &self,
        p: &mut ResolvedPlacement,
        graph: &RegionGraph,
        env: BuildingSiteEnvironment<'_>,
        key: PoseKey,
        epoch: Epoch,
    ) -> Verdict {
        #[cfg(test)]
        self.site_feasibility
            .solves
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let verdict = self
            .prepare_site_support(p, graph, env.road_surface, env.terrain)
            .map(|()| p.support_height_m);
        let dependencies = Arc::new(Dependencies::capture(self, p, env));
        let mut cache = self.site_feasibility.poses(key).lock().unwrap();
        let poses = cache.entry(p.asset_id.clone()).or_default();
        // Cursor previews have no persistent identity. Bound their memoization separately.
        if key.parcel == 0 && poses.len() >= 256 {
            poses.clear();
        }
        poses.insert(
            key,
            CachedSite {
                epoch,
                dependencies,
                verdict,
            },
        );
        verdict
    }

    /// Drops results for removed, occupied, rezoned or reattached parcels during hourly collection.
    pub(crate) fn prune_site_feasibility(&self, zoning: &ZoningSystem) {
        self.site_feasibility.zoning.prune(zoning);
        self.site_feasibility
            .entries
            .lock()
            .unwrap()
            .retain(|asset, poses| {
                let Some(building) = self
                    .registry
                    .get(asset)
                    .and_then(|e| e.manifest.building.as_ref())
                else {
                    return false;
                };
                poses.retain(|key, _| {
                    if key.width != building.lot_width_cells as usize
                        || key.depth != building.lot_depth_cells as usize
                    {
                        return false;
                    }
                    zoning.parcel_by_raw_id(key.parcel).is_some_and(|p| {
                        let center = p.front_center()
                            + p.normal() * (key.depth as f32 * f32::from_bits(key.bits[5]) * 0.5);
                        let facing = -p.normal();
                        p.is_available()
                            && p.edge_idx() == key.edge
                            && p.side() == key.side
                            && p.zone_profile_runtime_id() == key.profile
                            && p.frontage_center_t().to_bits() == key.bits[4]
                            && center.x.to_bits() == key.bits[0]
                            && center.y.to_bits() == key.bits[1]
                            && facing.x.to_bits() == key.bits[2]
                            && facing.y.to_bits() == key.bits[3]
                    })
                });
                !poses.is_empty()
            });
        self.site_feasibility
            .previews
            .lock()
            .unwrap()
            .retain(|asset, _| self.registry.get(asset).is_some());
    }

    #[cfg(test)]
    pub(crate) fn site_feasibility_solve_count(&self) -> u64 {
        self.site_feasibility
            .solves
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::graph::Edge;
    use crate::simulation::network::types::NodeType;

    #[test]
    fn road_dependencies_use_query_grid_and_invalidate_local_pieces() {
        let allocator = BuildingAllocator::new();
        let terrain = TerrainSystem::new(128, 128);
        let mut graph = RegionGraph::new();
        let centres = [-224.0, 224.0];
        for x in centres {
            let points = vec![
                Vector3::new(x - 20.0, 0.0, 0.0),
                Vector3::new(x + 20.0, 0.0, 0.0),
            ];
            let start = graph.add_node(points[0], NodeType::Junction);
            let end = graph.add_node(points[1], NodeType::Junction);
            graph.add_edge(Edge {
                start_node: start,
                end_node: end,
                primary_type: TransitType::Road,
                allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
                width: 7.0,
                fwd_lanes: 1,
                bkw_lanes: 1,
                physical_length: 40.0,
                geometry: points.clone(),
                physical_geometry: points,
                ..Edge::default()
            });
        }
        for (span, origin_x, origin_z) in [(32.0, 0.0, 0.0), (512.0, -10_000.0, 5_000.0)] {
            let mut surface = RoadSurfaceSystem::new_with_chunk_grid(span, origin_x, origin_z);
            for edge in 0..graph.edge_count() {
                surface.mark_edge_dirty(&graph, edge);
            }
            assert!(surface.compile_dirty(&graph, &terrain));
            for (local, x) in centres.into_iter().enumerate() {
                let bounds = (x - 24.0, -12.0, x + 24.0, 12.0);
                let (spans, nodes) = Dependencies::roads(bounds, &surface);
                assert_eq!(
                    spans.keys().copied().collect::<Vec<_>>(),
                    vec![local],
                    "render span={span}, origin=({origin_x},{origin_z}), road={local}"
                );
                assert_eq!(
                    nodes.keys().copied().collect::<Vec<_>>(),
                    vec![(local * 2) as u32, (local * 2 + 1) as u32]
                );
                let mut terrain_snapshot = TerrainVisualOverlay::new(&terrain);
                terrain_snapshot.capture_region(&terrain, bounds.0, bounds.1, bounds.2, bounds.3);
                let dependencies = Dependencies {
                    bounds,
                    terrain: terrain_snapshot,
                    spans,
                    nodes,
                    sites: allocator.terrain_site_snapshot_for_world_bounds(
                        bounds.0, bounds.1, bounds.2, bounds.3,
                    ),
                };
                let matches = |road_surface: &RoadSurfaceSystem| {
                    dependencies.matches(
                        &allocator,
                        BuildingSiteEnvironment {
                            road_surface,
                            terrain: &terrain,
                        },
                    )
                };
                assert!(matches(&surface));

                // Replace immutable published records without touching terrain, so road
                // dependency tracking alone must distinguish remote and local updates.
                let remote = 1 - local;
                let replacement = Arc::new(
                    surface.compiled_visual_span_pieces[&remote]
                        .as_ref()
                        .clone(),
                );
                surface
                    .compiled_visual_span_pieces
                    .insert(remote, replacement);
                assert!(
                    matches(&surface),
                    "remote road replacement must preserve local feasibility"
                );
                let replacement =
                    Arc::new(surface.compiled_visual_span_pieces[&local].as_ref().clone());
                let original = surface
                    .compiled_visual_span_pieces
                    .insert(local, replacement)
                    .unwrap();
                assert!(
                    !matches(&surface),
                    "local road replacement must invalidate feasibility"
                );
                surface.compiled_visual_span_pieces.insert(local, original);
                assert!(matches(&surface));

                let node = (local * 2) as u32;
                let replacement =
                    Arc::new(surface.compiled_visual_node_pieces[&node].as_ref().clone());
                let original = surface
                    .compiled_visual_node_pieces
                    .insert(node, replacement)
                    .unwrap();
                assert!(
                    !matches(&surface),
                    "local node replacement must invalidate feasibility"
                );
                surface.compiled_visual_node_pieces.insert(node, original);
            }
        }
    }
}
