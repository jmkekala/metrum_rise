// SPDX-License-Identifier: GPL-2.0-only

//! Shared, bounded site feasibility for demand and non-mutating zoning checks.

mod zoning;

use super::*;
use crate::simulation::buildings::allocator::BuildingSiteTerrainSnapshot;
use crate::simulation::network::roadbed::PieceId;
use crate::simulation::terrain::TerrainSourceCapture;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Live geometry inputs for the shared roadside-site solver.
#[derive(Clone, Copy)]
pub(crate) struct BuildingSiteEnvironment<'a> {
    /// Compiled road surfaces from the same graph revision used for placement.
    pub(crate) road_surface: &'a RoadbedSystem,
    /// Current source and engineered visual terrain.
    pub(crate) terrain: &'a TerrainSystem,
}

type Bounds = (f32, f32, f32, f32);
type Epoch = (u64, u64, u64, u64, u64);

// Growth of the box whose neighbouring sites a solve reads: the neighbour test's 5 cm contact
// tolerance, with a wide margin for the f32 rounding of the boxes.
const SITE_DEPENDENCY_SLACK_M: f32 = 1.0;
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

// What a solve read, by the generation that moves when it changes (`docs/terrain.md` T12 (a)):
// the base terrain the tie-in rays sample and the roads they and the nearest-road probes meet,
// within the reach of the lot (`bounds`), and the neighbouring sites the height test compares,
// which meet the lot itself (`site_bounds`).
struct Dependencies {
    bounds: Bounds,
    site_bounds: Bounds,
    terrain: TerrainSourceCapture,
    // Each roadbed piece in bounds with the generation that built it.
    roads: Vec<(PieceId, u64)>,
    sites: BuildingSiteTerrainSnapshot,
}

impl Dependencies {
    fn roads(bounds: Bounds, roadbed: &RoadbedSystem) -> Vec<(PieceId, u64)> {
        roadbed
            .pieces_in_bounds([
                f64::from(bounds.0),
                f64::from(bounds.1),
                f64::from(bounds.2),
                f64::from(bounds.3),
            ])
            .collect()
    }

    fn capture(
        allocator: &BuildingAllocator,
        p: &ResolvedPlacement,
        env: BuildingSiteEnvironment<'_>,
    ) -> Self {
        let lot = allocator.placement_site_bounds(p);
        let footprint = polygon_bounds(&allocator.placement_required_flat_support_footprint(p));
        let b = (
            lot.0.min(footprint.0),
            lot.1.min(footprint.1),
            lot.2.max(footprint.2),
            lot.3.max(footprint.3),
        );
        Self::capture_bounds(allocator, b, env)
    }

    // `b` holds the solve's support footprint.
    fn capture_bounds(
        allocator: &BuildingAllocator,
        b: Bounds,
        env: BuildingSiteEnvironment<'_>,
    ) -> Self {
        let grow = |m: f32| (b.0 - m, b.1 - m, b.2 + m, b.3 + m);
        let bounds = grow(super::super::site::site_feasibility_dependency_margin_m(env.terrain));
        let site_bounds = grow(SITE_DEPENDENCY_SLACK_M);
        let mut terrain = TerrainSourceCapture::new(env.terrain);
        terrain.capture_region(env.terrain, bounds.0, bounds.1, bounds.2, bounds.3);
        let roads = Self::roads(bounds, env.road_surface);
        let sites = Self::sites(site_bounds, allocator);
        Self {
            bounds,
            site_bounds,
            terrain,
            roads,
            sites,
        }
    }

    fn sites(b: Bounds, allocator: &BuildingAllocator) -> BuildingSiteTerrainSnapshot {
        allocator.terrain_site_snapshot_for_world_bounds(b.0, b.1, b.2, b.3)
    }

    // Whether the inputs whose generation moved between the epochs `then` (the capture's, or the
    // last check's) and `now` still hold their captured values: an unmoved generation leaves
    // its inputs unchanged, so only the moved ones are compared. The solve reads the base, not
    // the visual buffer.
    fn matches(
        &self,
        then: Epoch,
        now: Epoch,
        allocator: &BuildingAllocator,
        env: BuildingSiteEnvironment<'_>,
    ) -> bool {
        (then.0 == now.0 || self.terrain.matches_current(env.terrain))
            && (then.2 == now.2 || Self::roads(self.bounds, env.road_surface) == self.roads)
            && (then.3 == now.3 || Self::sites(self.site_bounds, allocator) == self.sites)
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
            && (self.epoch == epoch
                || self.dependencies.matches(self.epoch, epoch, allocator, env))
    }
}

// Stored-parcel shards of the site cache: the hourly lot scan looks poses up from every pool
// worker, each lot in the shard of its parcel, so lookups rarely wait on one another
// (`SIM-02-K`).
const SITE_CACHE_SHARDS: usize = 32;

/// Derived cache only: cloning an allocator starts empty so independent edits cannot share epochs.
#[derive(Default)]
pub(crate) struct SiteFeasibilityCache {
    entries: [Mutex<HashMap<String, HashMap<PoseKey, CachedSite>>>; SITE_CACHE_SHARDS],
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
            &self.entries[(key.parcel % SITE_CACHE_SHARDS as u64) as usize]
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
        if !env.road_surface.is_current() {
            return Err(DemandSpawnPlacementRejection::FrontageRoadSurfaceMissing);
        }
        let epoch = (
            env.terrain.source_generation(),
            env.terrain.visual_generation(),
            env.road_surface.generation(),
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
        for shard in &self.site_feasibility.entries {
            shard.lock().unwrap().retain(|asset, poses| {
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
        }
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
            let mut roadbed = RoadbedSystem::new_with_chunk_grid(span, origin_x, origin_z);
            for edge in 0..graph.edge_count() {
                roadbed.mark_edge_dirty(edge);
            }
            roadbed.compile_dirty(&graph);
            assert!(roadbed.is_current());
            for (local, x) in centres.into_iter().enumerate() {
                let bounds = (x - 24.0, -12.0, x + 24.0, 12.0);
                let roads = Dependencies::roads(bounds, &roadbed);
                let local_node = (local * 2) as u32;
                assert_eq!(
                    roads.iter().map(|(piece, _)| *piece).collect::<Vec<_>>(),
                    vec![
                        PieceId::Span(local),
                        PieceId::Node(local_node),
                        PieceId::Node(local_node + 1),
                    ],
                    "render span={span}, origin=({origin_x},{origin_z}), road={local}"
                );
                let mut terrain_snapshot = TerrainSourceCapture::new(&terrain);
                terrain_snapshot.capture_region(&terrain, bounds.0, bounds.1, bounds.2, bounds.3);
                let dependencies = Dependencies {
                    bounds,
                    site_bounds: bounds,
                    terrain: terrain_snapshot,
                    roads,
                    sites: Dependencies::sites(bounds, &allocator),
                };
                // Every generation moved, so every input is compared.
                let matches = |road_surface: &RoadbedSystem| {
                    dependencies.matches(
                        (0, 0, 0, 0, 0),
                        (1, 1, 1, 1, 0),
                        &allocator,
                        BuildingSiteEnvironment {
                            road_surface,
                            terrain: &terrain,
                        },
                    )
                };
                assert!(matches(&roadbed));

                // Rebuild pieces without touching terrain, so road dependency tracking alone
                // must distinguish remote and local updates.
                let mut remote = roadbed.clone();
                remote.mark_edge_dirty(1 - local);
                remote.compile_dirty(&graph);
                assert!(
                    matches(&remote),
                    "remote road rebuild must preserve local feasibility"
                );
                let mut rebuilt = roadbed.clone();
                rebuilt.mark_edge_dirty(local);
                rebuilt.compile_dirty(&graph);
                assert!(
                    !matches(&rebuilt),
                    "local road rebuild must invalidate feasibility"
                );
            }
        }
    }
}
