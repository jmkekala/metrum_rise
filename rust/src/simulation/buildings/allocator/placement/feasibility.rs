// SPDX-License-Identifier: GPL-2.0-only

//! Shared, bounded site feasibility for demand and non-mutating zoning checks.

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
        let x = |v: f32| ((v - surface.chunk_origin_x_m) / surface.chunk_span_m).floor() as i32;
        let z = |v: f32| ((v - surface.chunk_origin_z_m) / surface.chunk_span_m).floor() as i32;
        for cx in x(bounds.0)..=x(bounds.2) {
            for cz in z(bounds.1)..=z(bounds.3) {
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

struct CachedSite {
    epoch: Epoch,
    dependencies: Arc<Dependencies>,
    verdict: Verdict,
}

/// Derived cache only: cloning an allocator starts empty so independent edits cannot share epochs.
#[derive(Default)]
pub(crate) struct SiteFeasibilityCache {
    entries: Mutex<HashMap<String, HashMap<PoseKey, CachedSite>>>,
    previews: Mutex<HashMap<String, HashMap<PoseKey, CachedSite>>>,
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
    /// Tests at least one legal initial asset without inserting a parcel, building or terrain edit.
    /// Existing occupied parcels are evaluated as redevelopment, excluding only their own site.
    pub(crate) fn zoning_site_feasibility(
        &self,
        geometry: &ParcelGeometry,
        profile_id: u16,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        env: BuildingSiteEnvironment<'_>,
    ) -> Result<(), &'static str> {
        if self.parcel_geometry_overlaps_explicit_site(geometry) {
            return Err("parcel overlaps a placed building site");
        }
        if profile_id == 0 {
            return Ok(());
        }
        let profile = zoning
            .profiles
            .profile_by_runtime_id(profile_id)
            .ok_or("unknown zoning profile")?;
        let zone_class =
            zone_type_to_zone_class(profile.zone_type).ok_or("unsupported zoning profile")?;
        let existing = zoning.parcel_at(geometry.center);
        let parcel = ZoningParcel::new(
            existing.map(|p| p.id()).unwrap_or_default(),
            *geometry,
            profile_id,
        );
        let mut reason = "no compatible initial building asset fits this parcel";
        for id in self
            .registry
            .buildings_for_zone_density(zone_class, profile.density.as_str())
        {
            let Some(params) = self.asset_placement_params(id, catalog) else {
                continue;
            };
            if params.initial_level != 1 {
                continue;
            }
            let Some(mut p) = self.resolve_slot_replacing(
                id,
                &params,
                &parcel,
                zoning,
                graph,
                existing.and_then(|p| p.occupied_building()),
            ) else {
                continue;
            };
            match self.prepare_site_support_cached(&mut p, graph, env) {
                Ok(()) => return Ok(()),
                Err(DemandSpawnPlacementRejection::SiteSupportTieInInvalid) => {
                    reason =
                        "flat yard cannot connect to road and terrain within the grading envelope"
                }
                Err(_) => {
                    reason =
                        "building site cannot connect to the frontage road or neighboring sites"
                }
            }
        }
        Err(reason)
    }

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
                .map(|old| (old.epoch, Arc::clone(&old.dependencies), old.verdict))
        };
        let verdict = if let Some((old_epoch, dependencies, verdict)) =
            previous.filter(|(e, _, _)| e.4 == epoch.4)
        {
            if old_epoch == epoch || dependencies.matches(self, env) {
                if old_epoch != epoch
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
                verdict
            } else {
                self.cache_site_solution(p, graph, env, key, epoch)
            }
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
