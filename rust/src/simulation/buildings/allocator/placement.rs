//! Demand-driven building placement candidate discovery and frontage-slot resolution.

use crate::assets::AnchorType;
use crate::assets::asset::PlacementMode;
use crate::config::SIDEWALK_WIDTH;
use crate::debug_log;
use crate::simulation::buildings::allocator::{
    Building, BuildingAllocator, DemandSpawnPlacementRejection, ExplicitServicePlacementRejection,
    baseline_private_zone_slot, resolve_building_economy_profile_binding_with_catalog,
    zone_class_to_zone_type, zone_type_to_zone_class,
};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntimeKind, RuntimeEconomyCatalog, RuntimeEconomyTuning,
};
use crate::simulation::economy::demand::{
    DemandSpawnAction, DemandSpawnCandidate, DemandSpawnCandidatesByUse,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::RoadSurfaceSystem;
use crate::simulation::network::types::{TransitFlags, TransitType};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::work_area::initial_work_area_scale;
use crate::simulation::zoning::{ParcelGeometry, ZoneType, ZoningParcel, ZoningSystem};
use godot::prelude::{Vector2, Vector3};
use rayon::prelude::*;
use std::collections::BTreeMap;

use super::site::building_site_support_tie_in_is_valid;

const EXPLICIT_SERVICE_FRONTAGE_MIN_SEARCH_M: f32 = 50.0;
const EXPLICIT_SERVICE_FRONTAGE_SEARCH_MARGIN_M: f32 = 20.0;
const EXPLICIT_SERVICE_SITE_OVERLAP_EPS_M: f32 = 0.05;
const EXPLICIT_SERVICE_ROAD_OVERLAP_QUERY_PAD_M: f32 = 128.0;

impl BuildingAllocator {
    pub(crate) fn collect_demand_spawn_candidates_by_use(
        &self,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
    ) -> DemandSpawnCandidatesByUse {
        let asset_candidates_by_profile =
            self.collect_spawn_asset_candidates_by_profile(zoning, catalog);
        let candidates = zoning
            .parcels()
            .par_iter()
            .fold(
                DemandSpawnCandidateSortBuckets::default,
                |mut candidates, parcel| {
                    let edge_idx = parcel.edge_idx();
                    if edge_idx >= graph.edge_count() {
                        return candidates;
                    }
                    let edge = graph.edge(edge_idx);
                    if edge.deleted
                        || edge.no_building_spawn
                        || edge.physical_length < 0.1
                        || edge.physical_geometry.len() < 2
                        || !parcel.is_available()
                    {
                        return candidates;
                    }

                    let profile_runtime_id = parcel.zone_profile_runtime_id();
                    let Some(profile) = zoning.profiles.profile_by_runtime_id(profile_runtime_id)
                    else {
                        return candidates;
                    };
                    let zone_type = profile.zone_type;

                    let Some(profile_candidates) =
                        asset_candidates_by_profile.get(&profile_runtime_id)
                    else {
                        return candidates;
                    };

                    let Some(resolved) = self.select_deterministic_fresh_spawn_asset(
                        profile_candidates,
                        profile_runtime_id,
                        parcel,
                        zoning,
                        graph,
                    ) else {
                        return candidates;
                    };

                    let sort_key = DemandSpawnCandidateSortKey::from_resolved(&resolved);
                    candidates.push_zone_type(
                        zone_type,
                        sort_key,
                        DemandSpawnCandidate {
                            action: DemandSpawnAction {
                                parcel_id: parcel.id().raw(),
                                asset_id: resolved.asset_id,
                            },
                            density: profile.density.as_str().to_owned(),
                        },
                    );
                    candidates
                },
            )
            .reduce(
                DemandSpawnCandidateSortBuckets::default,
                |mut left, right| {
                    left.extend(right);
                    left
                },
            );

        let candidates = candidates.finish();
        debug_log!(
            "spawn",
            "collect_candidates_by_use: residential={} commercial={} industrial={}",
            candidates.residential.len(),
            candidates.commercial.len(),
            candidates.industrial.len(),
        );
        candidates
    }

    fn collect_spawn_asset_candidates_by_profile(
        &self,
        zoning: &ZoningSystem,
        catalog: &RuntimeEconomyCatalog,
    ) -> BTreeMap<u16, SpawnProfileAssetCandidates> {
        let mut by_profile = BTreeMap::new();
        for profile in zoning.profiles.profiles() {
            let Some(zone_class) = zone_type_to_zone_class(profile.zone_type) else {
                continue;
            };
            let mut candidates = Vec::new();
            for qualified_id in self
                .registry
                .buildings_for_zone_density(zone_class, profile.density.as_str())
            {
                let Some(entry) = self.registry.get(qualified_id) else {
                    continue;
                };
                let Some(building) = entry.manifest.building.as_ref() else {
                    continue;
                };
                if !building.is_zoned_private() || building.level != 1 {
                    continue;
                }
                let Some(params) = self.asset_placement_params(qualified_id, catalog) else {
                    continue;
                };
                if !zoning.profiles.asset_is_legal(
                    profile.runtime_id,
                    params.zone_type,
                    &params.density,
                    &params.tags,
                ) {
                    continue;
                }
                let family_key = entry
                    .manifest
                    .asset_set
                    .clone()
                    .unwrap_or_else(|| qualified_id.clone());
                candidates.push(SpawnAssetCandidate {
                    family_key,
                    qualified_id: qualified_id.clone(),
                    params,
                });
            }
            candidates.sort_by(|left, right| {
                left.family_key
                    .cmp(&right.family_key)
                    .then(left.qualified_id.cmp(&right.qualified_id))
            });
            by_profile.insert(
                profile.runtime_id,
                SpawnProfileAssetCandidates { candidates },
            );
        }
        by_profile
    }

    fn select_deterministic_fresh_spawn_asset(
        &self,
        profile_candidates: &SpawnProfileAssetCandidates,
        profile_runtime_id: u16,
        parcel: &ZoningParcel,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
    ) -> Option<ResolvedPlacement> {
        if profile_candidates.candidates.is_empty() {
            return None;
        }

        let mut best = None;
        for candidate in &profile_candidates.candidates {
            if let Some(resolved) = self.resolve_slot(
                &candidate.qualified_id,
                &candidate.params,
                parcel,
                zoning,
                graph,
            ) {
                let selection_key = (
                    stable_strip_family_hash(
                        profile_runtime_id,
                        parcel.id().raw(),
                        &candidate.family_key,
                    ),
                    candidate.family_key.as_str(),
                    stable_site_variant_hash(
                        profile_runtime_id,
                        parcel.id().raw(),
                        &candidate.qualified_id,
                    ),
                    candidate.qualified_id.as_str(),
                );
                if best
                    .as_ref()
                    .map(|(best_key, _)| selection_key < *best_key)
                    .unwrap_or(true)
                {
                    best = Some((selection_key, resolved));
                }
            }
        }

        best.map(|(_, resolved)| resolved)
    }

    fn asset_placement_params(
        &self,
        asset_id: &str,
        catalog: &RuntimeEconomyCatalog,
    ) -> Option<AssetPlacementParams> {
        let entry = self.registry.get(asset_id)?;
        let building = entry.manifest.building.as_ref()?;
        if !building.is_zoned_private() {
            return None;
        }
        let zone_type = zone_class_to_zone_type(building.zone_type?);
        let economy_binding = resolve_building_economy_profile_binding_with_catalog(
            &self.registry,
            catalog,
            asset_id,
        );
        if matches!(zone_type, ZoneType::Commercial | ZoneType::Industrial)
            && (economy_binding.economy_broken || economy_binding.runtime_id == 0)
        {
            debug_log!(
                "spawn",
                "asset_params: {} rejected — economy_broken={} runtime_id={}",
                asset_id,
                economy_binding.economy_broken,
                economy_binding.runtime_id,
            );
            return None;
        }
        Some(AssetPlacementParams {
            zone_type,
            density: building.density_key()?.to_owned(),
            tags: entry.manifest.tags.clone(),
            width_cells: building.lot_width_cells as usize,
            depth_cells: building.lot_depth_cells as usize,
            initial_level: building.level,
        })
    }

    fn resolve_slot(
        &self,
        asset_id: &str,
        params: &AssetPlacementParams,
        parcel: &ZoningParcel,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
    ) -> Option<ResolvedPlacement> {
        let edge_idx = parcel.edge_idx();
        let edge = graph.edge(edge_idx);
        let edge_width = edge.width;
        let zone_cell_m = zoning.config.zone_cell_m;
        if parcel.occupied_building().is_some() || edge.deleted || edge.no_building_spawn {
            return None;
        }

        let frontage_profile_runtime_id = parcel.zone_profile_runtime_id();
        if frontage_profile_runtime_id == 0 {
            return None;
        }
        if !zoning.profiles.asset_is_legal(
            frontage_profile_runtime_id,
            params.zone_type,
            &params.density,
            &params.tags,
        ) {
            return None;
        }

        let width_m = params.width_cells as f32 * zone_cell_m;
        let depth_m = params.depth_cells as f32 * zone_cell_m;
        if width_m > parcel.frontage_m() + f32::EPSILON || depth_m > parcel.depth_m() + f32::EPSILON
        {
            return None;
        }

        let center_2d = parcel.front_center() + parcel.normal() * (depth_m * 0.5);

        let placement = ResolvedPlacement {
            asset_id: asset_id.to_owned(),
            zone_profile_runtime_id: frontage_profile_runtime_id,
            zone_type: params.zone_type,
            initial_level: params.initial_level,
            parcel_id: parcel.id().raw(),
            edge_idx,
            side: parcel.side(),
            cell_x: 0,
            width_cells: params.width_cells,
            depth_cells: params.depth_cells,
            zone_cell_m,
            center_2d,
            support_height_m: 0.0,
            facing_dir: parcel.normal() * -1.0,
            frontage_t: parcel.frontage_center_t(),
            edge_width,
        };
        if self.placement_overlaps_existing_flat_support(&placement) {
            return None;
        }
        Some(placement)
    }

    fn commit_resolved_slot(
        &mut self,
        placement: ResolvedPlacement,
        zoning: &mut ZoningSystem,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> usize {
        let parcel_id = placement.parcel_id;
        let building_idx = self.place_building_instance(placement, catalog, tuning);
        zoning.occupy_parcel(parcel_id, building_idx);
        self.bump_building_ref_revision();
        self.dirty = true;
        self.dirty_index = true;
        self.entrances_dirty = true;
        if let Some(zone_idx) = baseline_private_zone_slot(self.buildings[building_idx].zone_type) {
            self.dirty_zones[zone_idx] = true;
        }
        debug_log!(
            "economy",
            "demand placed building idx={} asset_id={} zone={:?} edge={} cell=({}, {}) center=({:.1}, {:.1}) support_height_m={:.2} site_surfaces={}",
            building_idx,
            self.buildings[building_idx].asset_id,
            self.buildings[building_idx].zone_type,
            self.buildings[building_idx].edge_idx,
            self.buildings[building_idx].cell_x,
            self.buildings[building_idx].cell_y,
            self.buildings[building_idx].center_x,
            self.buildings[building_idx].center_y,
            self.buildings[building_idx].support_height_m,
            self.building_sites[building_idx].surface_debug_summary(),
        );
        building_idx
    }

    pub(crate) fn execute_demand_spawn_action(
        &mut self,
        action: &DemandSpawnAction,
        zoning: &mut ZoningSystem,
        graph: &RegionGraph,
        road_surface: &RoadSurfaceSystem,
        terrain: &TerrainSystem,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> Result<usize, DemandSpawnPlacementRejection> {
        let Some(params) = self.asset_placement_params(&action.asset_id, catalog) else {
            return Err(DemandSpawnPlacementRejection::AssetUnavailable);
        };
        let Some(parcel) = zoning.parcel_by_raw_id(action.parcel_id) else {
            return Err(DemandSpawnPlacementRejection::ParcelUnavailable);
        };
        let Some(mut resolved) =
            self.resolve_slot(&action.asset_id, &params, parcel, zoning, graph)
        else {
            return Err(DemandSpawnPlacementRejection::SlotUnavailable);
        };
        resolved.support_height_m =
            self.resolve_site_support_height(&resolved, graph, road_surface, terrain)?;
        self.validate_site_support_tie_in(&resolved, graph, road_surface, terrain)?;
        Ok(self.commit_resolved_slot(resolved, zoning, catalog, tuning))
    }

    pub(crate) fn preview_explicit_service_placement(
        &self,
        asset_id: &str,
        point: Vector2,
        zone_cell_m: f32,
        graph: &RegionGraph,
        road_surface: &RoadSurfaceSystem,
        terrain: &TerrainSystem,
        catalog: &RuntimeEconomyCatalog,
    ) -> Result<ExplicitServicePlacementPreview, ExplicitServicePlacementRejection> {
        let params = self.explicit_service_placement_params(asset_id, catalog)?;
        let mut placement =
            self.resolve_explicit_service_placement(asset_id, &params, point, zone_cell_m, graph)?;
        placement.support_height_m = self
            .resolve_site_support_height(&placement, graph, road_surface, terrain)
            .map_err(explicit_rejection_from_site_rejection)?;
        if let Err(rejection) = self.validate_explicit_site_overlap(&placement) {
            return Ok(ExplicitServicePlacementPreview {
                corners: self.placement_site_corners(&placement),
                center_2d: placement.center_2d,
                support_height_m: placement.support_height_m,
                facing_dir: placement.facing_dir,
                valid: false,
                rejection: Some(rejection),
            });
        }
        if let Err(rejection) = self.validate_explicit_site_road_overlap(&placement, graph) {
            return Ok(ExplicitServicePlacementPreview {
                corners: self.placement_site_corners(&placement),
                center_2d: placement.center_2d,
                support_height_m: placement.support_height_m,
                facing_dir: placement.facing_dir,
                valid: false,
                rejection: Some(rejection),
            });
        }
        if let Err(rejection) = self
            .validate_site_support_tie_in(&placement, graph, road_surface, terrain)
            .map_err(explicit_rejection_from_site_rejection)
        {
            return Ok(ExplicitServicePlacementPreview {
                corners: self.placement_site_corners(&placement),
                center_2d: placement.center_2d,
                support_height_m: placement.support_height_m,
                facing_dir: placement.facing_dir,
                valid: false,
                rejection: Some(rejection),
            });
        }
        Ok(ExplicitServicePlacementPreview {
            corners: self.placement_site_corners(&placement),
            center_2d: placement.center_2d,
            support_height_m: placement.support_height_m,
            facing_dir: placement.facing_dir,
            valid: true,
            rejection: None,
        })
    }

    pub(crate) fn execute_explicit_service_placement(
        &mut self,
        asset_id: &str,
        point: Vector2,
        zone_cell_m: f32,
        graph: &RegionGraph,
        road_surface: &RoadSurfaceSystem,
        terrain: &TerrainSystem,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> Result<usize, ExplicitServicePlacementRejection> {
        let params = self.explicit_service_placement_params(asset_id, catalog)?;
        let mut placement =
            self.resolve_explicit_service_placement(asset_id, &params, point, zone_cell_m, graph)?;
        placement.support_height_m = self
            .resolve_site_support_height(&placement, graph, road_surface, terrain)
            .map_err(explicit_rejection_from_site_rejection)?;
        self.validate_explicit_site_overlap(&placement)?;
        self.validate_explicit_site_road_overlap(&placement, graph)?;
        self.validate_site_support_tie_in(&placement, graph, road_surface, terrain)
            .map_err(explicit_rejection_from_site_rejection)?;

        let building_idx = self.place_building_instance(placement, catalog, tuning);
        self.bump_building_ref_revision();
        self.dirty = true;
        self.dirty_index = true;
        self.entrances_dirty = true;
        self.accumulate_pending_site_dirty_bounds(self.site_world_bounds(building_idx));
        debug_log!(
            "economy",
            "explicit service placed building idx={} asset_id={} edge={} center=({:.1}, {:.1}) support_height_m={:.2}",
            building_idx,
            self.buildings[building_idx].asset_id,
            self.buildings[building_idx].edge_idx,
            self.buildings[building_idx].center_x,
            self.buildings[building_idx].center_y,
            self.buildings[building_idx].support_height_m,
        );
        Ok(building_idx)
    }

    pub(crate) fn preview_explicit_industry_placement(
        &self,
        asset_id: &str,
        point: Vector2,
        zone_cell_m: f32,
        graph: &RegionGraph,
        road_surface: &RoadSurfaceSystem,
        terrain: &TerrainSystem,
        catalog: &RuntimeEconomyCatalog,
    ) -> Result<ExplicitServicePlacementPreview, ExplicitServicePlacementRejection> {
        let params = self.explicit_industry_placement_params(asset_id, catalog)?;
        let mut placement =
            self.resolve_explicit_service_placement(asset_id, &params, point, zone_cell_m, graph)?;
        placement.support_height_m = self
            .resolve_site_support_height(&placement, graph, road_surface, terrain)
            .map_err(explicit_rejection_from_site_rejection)?;
        if let Err(rejection) = self.validate_explicit_site_overlap(&placement) {
            return Ok(ExplicitServicePlacementPreview {
                corners: self.placement_site_corners(&placement),
                center_2d: placement.center_2d,
                support_height_m: placement.support_height_m,
                facing_dir: placement.facing_dir,
                valid: false,
                rejection: Some(rejection),
            });
        }
        if let Err(rejection) = self.validate_explicit_site_road_overlap(&placement, graph) {
            return Ok(ExplicitServicePlacementPreview {
                corners: self.placement_site_corners(&placement),
                center_2d: placement.center_2d,
                support_height_m: placement.support_height_m,
                facing_dir: placement.facing_dir,
                valid: false,
                rejection: Some(rejection),
            });
        }
        if let Err(rejection) = self
            .validate_site_support_tie_in(&placement, graph, road_surface, terrain)
            .map_err(explicit_rejection_from_site_rejection)
        {
            return Ok(ExplicitServicePlacementPreview {
                corners: self.placement_site_corners(&placement),
                center_2d: placement.center_2d,
                support_height_m: placement.support_height_m,
                facing_dir: placement.facing_dir,
                valid: false,
                rejection: Some(rejection),
            });
        }
        Ok(ExplicitServicePlacementPreview {
            corners: self.placement_site_corners(&placement),
            center_2d: placement.center_2d,
            support_height_m: placement.support_height_m,
            facing_dir: placement.facing_dir,
            valid: true,
            rejection: None,
        })
    }

    pub(crate) fn execute_explicit_industry_placement(
        &mut self,
        asset_id: &str,
        point: Vector2,
        zone_cell_m: f32,
        graph: &RegionGraph,
        road_surface: &RoadSurfaceSystem,
        terrain: &TerrainSystem,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> Result<usize, ExplicitServicePlacementRejection> {
        let params = self.explicit_industry_placement_params(asset_id, catalog)?;
        let mut placement =
            self.resolve_explicit_service_placement(asset_id, &params, point, zone_cell_m, graph)?;
        placement.support_height_m = self
            .resolve_site_support_height(&placement, graph, road_surface, terrain)
            .map_err(explicit_rejection_from_site_rejection)?;
        self.validate_explicit_site_overlap(&placement)?;
        self.validate_explicit_site_road_overlap(&placement, graph)?;
        self.validate_site_support_tie_in(&placement, graph, road_surface, terrain)
            .map_err(explicit_rejection_from_site_rejection)?;

        let building_idx = self.place_building_instance(placement, catalog, tuning);
        self.bump_building_ref_revision();
        self.dirty = true;
        self.dirty_index = true;
        self.entrances_dirty = true;
        self.accumulate_pending_site_dirty_bounds(self.site_world_bounds(building_idx));
        debug_log!(
            "economy",
            "explicit industry placed building idx={} asset_id={} edge={} center=({:.1}, {:.1}) support_height_m={:.2}",
            building_idx,
            self.buildings[building_idx].asset_id,
            self.buildings[building_idx].edge_idx,
            self.buildings[building_idx].center_x,
            self.buildings[building_idx].center_y,
            self.buildings[building_idx].support_height_m,
        );
        Ok(building_idx)
    }

    fn explicit_service_placement_params(
        &self,
        asset_id: &str,
        catalog: &RuntimeEconomyCatalog,
    ) -> Result<AssetPlacementParams, ExplicitServicePlacementRejection> {
        let entry = self
            .registry
            .get(asset_id)
            .ok_or(ExplicitServicePlacementRejection::AssetUnavailable)?;
        let building = entry
            .manifest
            .building
            .as_ref()
            .ok_or(ExplicitServicePlacementRejection::NotServiceBuilding)?;
        if !self.registry.is_city_service_asset(asset_id) {
            return Err(ExplicitServicePlacementRejection::NotServiceBuilding);
        }

        let service_class = self
            .registry
            .service_class(asset_id)
            .ok_or(ExplicitServicePlacementRejection::NotServiceBuilding)?;
        if let Some(expected_service) = expected_utility_service_for_class(service_class) {
            let economy_binding = resolve_building_economy_profile_binding_with_catalog(
                &self.registry,
                catalog,
                asset_id,
            );
            let profile = catalog
                .profile_by_runtime_id(economy_binding.runtime_id)
                .filter(|profile| {
                    !economy_binding.economy_broken
                        && profile.utility_service.as_deref() == Some(expected_service)
                        && matches!(
                            profile.kind,
                            EconomyProfileRuntimeKind::UtilityProducer
                                | EconomyProfileRuntimeKind::UtilityProcessor
                        )
                })
                .ok_or(ExplicitServicePlacementRejection::UtilityProfileUnavailable)?;
            if profile.worker_capacity == 0 {
                return Err(ExplicitServicePlacementRejection::UtilityProfileUnavailable);
            }
        }

        Ok(AssetPlacementParams {
            zone_type: ZoneType::None,
            density: String::new(),
            tags: entry.manifest.tags.clone(),
            width_cells: building.lot_width_cells as usize,
            depth_cells: building.lot_depth_cells as usize,
            initial_level: building.level,
        })
    }

    fn explicit_industry_placement_params(
        &self,
        asset_id: &str,
        catalog: &RuntimeEconomyCatalog,
    ) -> Result<AssetPlacementParams, ExplicitServicePlacementRejection> {
        let entry = self
            .registry
            .get(asset_id)
            .ok_or(ExplicitServicePlacementRejection::AssetUnavailable)?;
        let building = entry
            .manifest
            .building
            .as_ref()
            .ok_or(ExplicitServicePlacementRejection::NotIndustryBuilding)?;
        if !self.registry.is_industry_area_asset(asset_id) {
            return Err(ExplicitServicePlacementRejection::NotIndustryBuilding);
        }
        let economy_binding = resolve_building_economy_profile_binding_with_catalog(
            &self.registry,
            catalog,
            asset_id,
        );
        let profile = catalog
            .profile_by_runtime_id(economy_binding.runtime_id)
            .filter(|_| !economy_binding.economy_broken)
            .ok_or_else(|| {
                if self.registry.is_field_producer_asset(asset_id) {
                    ExplicitServicePlacementRejection::FieldProfileUnavailable
                } else {
                    ExplicitServicePlacementRejection::ExtractorProfileUnavailable
                }
            })?;
        if let Some(resource_id) = self.registry.extractor_resource(asset_id) {
            let Some(resource_runtime_id) = catalog.resource_runtime_id_for_id(resource_id) else {
                return Err(ExplicitServicePlacementRejection::ExtractorProfileUnavailable);
            };
            if profile.kind != EconomyProfileRuntimeKind::Extractor
                || profile.worker_capacity == 0
                || profile.output_port(resource_runtime_id).is_none()
            {
                return Err(ExplicitServicePlacementRejection::ExtractorProfileUnavailable);
            }
        } else if let Some(resource_id) = self.registry.field_resource(asset_id) {
            let Some(resource_runtime_id) = catalog.resource_runtime_id_for_id(resource_id) else {
                return Err(ExplicitServicePlacementRejection::FieldProfileUnavailable);
            };
            if profile.kind != EconomyProfileRuntimeKind::FieldProducer
                || profile.worker_capacity == 0
                || profile.output_port(resource_runtime_id).is_none()
            {
                return Err(ExplicitServicePlacementRejection::FieldProfileUnavailable);
            }
        } else {
            return Err(ExplicitServicePlacementRejection::NotIndustryBuilding);
        }
        if profile.outputs.is_empty() {
            return Err(if self.registry.is_field_producer_asset(asset_id) {
                ExplicitServicePlacementRejection::FieldProfileUnavailable
            } else {
                ExplicitServicePlacementRejection::ExtractorProfileUnavailable
            });
        }

        Ok(AssetPlacementParams {
            zone_type: ZoneType::None,
            density: String::new(),
            tags: entry.manifest.tags.clone(),
            width_cells: building.lot_width_cells as usize,
            depth_cells: building.lot_depth_cells as usize,
            initial_level: building.level,
        })
    }

    fn resolve_explicit_service_placement(
        &self,
        asset_id: &str,
        params: &AssetPlacementParams,
        point: Vector2,
        zone_cell_m: f32,
        graph: &RegionGraph,
    ) -> Result<ResolvedPlacement, ExplicitServicePlacementRejection> {
        let width_m = params.width_cells as f32 * zone_cell_m;
        let depth_m = params.depth_cells as f32 * zone_cell_m;
        if width_m <= 0.0 || depth_m <= 0.0 {
            return Err(ExplicitServicePlacementRejection::RoadFrontageUnavailable);
        }

        let projection = self
            .closest_explicit_service_frontage(point, width_m, depth_m, graph)
            .ok_or(ExplicitServicePlacementRejection::RoadFrontageUnavailable)?;
        let edge = graph
            .get_edge(projection.edge_idx)
            .ok_or(ExplicitServicePlacementRejection::RoadFrontageUnavailable)?;
        let centerline = Self::sample_pos_on_edge(graph, projection.edge_idx, projection.t);
        let tangent = Self::sample_tangent_on_edge(graph, projection.edge_idx, projection.t);
        if tangent.length_squared() <= 1e-12 {
            return Err(ExplicitServicePlacementRejection::RoadFrontageUnavailable);
        }
        let outward = Vector2::new(tangent.y, -tangent.x).normalized() * projection.side as f32;
        let frontage_center = centerline + outward * (edge.width * 0.5 + SIDEWALK_WIDTH);
        let center_2d = frontage_center + outward * (depth_m * 0.5);

        Ok(ResolvedPlacement {
            asset_id: asset_id.to_owned(),
            zone_profile_runtime_id: 0,
            zone_type: params.zone_type,
            initial_level: params.initial_level,
            parcel_id: 0,
            edge_idx: projection.edge_idx,
            side: projection.side,
            cell_x: 0,
            width_cells: params.width_cells,
            depth_cells: params.depth_cells,
            zone_cell_m,
            center_2d,
            support_height_m: 0.0,
            facing_dir: -outward,
            frontage_t: projection.t,
            edge_width: edge.width,
        })
    }

    fn closest_explicit_service_frontage(
        &self,
        point: Vector2,
        width_m: f32,
        depth_m: f32,
        graph: &RegionGraph,
    ) -> Option<super::geometry::RoadFrontageProjection> {
        let search_radius = (width_m * 0.5 + depth_m + EXPLICIT_SERVICE_FRONTAGE_SEARCH_MARGIN_M)
            .max(EXPLICIT_SERVICE_FRONTAGE_MIN_SEARCH_M);
        let mut candidates =
            graph.get_edges_near_point(Vector3::new(point.x, 0.0, point.y), search_radius);
        candidates.sort_unstable();
        candidates.dedup();

        let mut best: Option<super::geometry::RoadFrontageProjection> = None;
        for edge_idx in candidates {
            let Some(edge) = graph.get_edge(edge_idx) else {
                continue;
            };
            if edge.deleted
                || edge.no_building_spawn
                || edge.physical_length < width_m.max(1.0)
                || edge.physical_geometry.len() < 2
            {
                continue;
            }
            let Some(projection) = Self::project_point_to_edge_centerline(edge_idx, edge, point)
            else {
                continue;
            };
            let half_width_t = (width_m * 0.5 / edge.physical_length).clamp(0.0, 0.49);
            let t = projection.t.clamp(half_width_t, 1.0 - half_width_t);
            let centerline = Self::sample_pos_on_edge(graph, edge_idx, t);
            let dist_sq = centerline.distance_squared_to(point);
            if dist_sq > search_radius * search_radius {
                continue;
            }
            let candidate = super::geometry::RoadFrontageProjection {
                edge_idx,
                t,
                side: projection.side,
                dist_sq,
            };
            if best.as_ref().is_none_or(|best| {
                candidate
                    .dist_sq
                    .total_cmp(&best.dist_sq)
                    .then(candidate.edge_idx.cmp(&best.edge_idx))
                    .is_lt()
            }) {
                best = Some(candidate);
            }
        }
        best
    }

    fn validate_explicit_site_overlap(
        &self,
        placement: &ResolvedPlacement,
    ) -> Result<(), ExplicitServicePlacementRejection> {
        let (min_x, min_z, max_x, max_z) = self.placement_site_bounds(placement);
        for building_idx in
            self.neighbor_site_candidate_indices(placement, min_x, min_z, max_x, max_z)
        {
            let Some((site_min_x, site_min_z, site_max_x, site_max_z)) =
                self.site_world_bounds(building_idx)
            else {
                continue;
            };
            let overlaps = site_min_x < max_x - EXPLICIT_SERVICE_SITE_OVERLAP_EPS_M
                && site_max_x > min_x + EXPLICIT_SERVICE_SITE_OVERLAP_EPS_M
                && site_min_z < max_z - EXPLICIT_SERVICE_SITE_OVERLAP_EPS_M
                && site_max_z > min_z + EXPLICIT_SERVICE_SITE_OVERLAP_EPS_M;
            if overlaps {
                return Err(ExplicitServicePlacementRejection::SiteOverlap);
            }
        }
        Ok(())
    }

    fn validate_explicit_site_road_overlap(
        &self,
        placement: &ResolvedPlacement,
        graph: &RegionGraph,
    ) -> Result<(), ExplicitServicePlacementRejection> {
        let site_corners = self.placement_site_corners(placement);
        let (min_x, min_z, max_x, max_z) = polygon_bounds(&site_corners);
        let nearby_edges = graph.get_edges_near_aabb(
            Vector3::new(
                min_x - EXPLICIT_SERVICE_ROAD_OVERLAP_QUERY_PAD_M,
                0.0,
                min_z - EXPLICIT_SERVICE_ROAD_OVERLAP_QUERY_PAD_M,
            ),
            Vector3::new(
                max_x + EXPLICIT_SERVICE_ROAD_OVERLAP_QUERY_PAD_M,
                0.0,
                max_z + EXPLICIT_SERVICE_ROAD_OVERLAP_QUERY_PAD_M,
            ),
        );
        for edge_idx in nearby_edges {
            let Some(edge) = graph.get_edge(edge_idx) else {
                continue;
            };
            if edge.deleted || edge.physical_geometry.len() < 2 {
                continue;
            }
            let road_half_width = road_collision_half_width_m(edge);
            for segment in edge.physical_geometry.windows(2) {
                let start = Vector2::new(segment[0].x, segment[0].z);
                let end = Vector2::new(segment[1].x, segment[1].z);
                let delta = end - start;
                if delta.length_squared()
                    <= EXPLICIT_SERVICE_SITE_OVERLAP_EPS_M * EXPLICIT_SERVICE_SITE_OVERLAP_EPS_M
                {
                    continue;
                }
                let tangent = delta.normalized();
                let normal = Vector2::new(tangent.y, -tangent.x);
                let road_corners = [
                    start - normal * road_half_width,
                    end - normal * road_half_width,
                    end + normal * road_half_width,
                    start + normal * road_half_width,
                ];
                if flat_support_footprints_overlap(
                    &site_corners,
                    &road_corners,
                    EXPLICIT_SERVICE_SITE_OVERLAP_EPS_M,
                ) {
                    debug_log!(
                        "economy",
                        "explicit service placement rejected: asset={} site overlaps road edge={} bounds=({:.1},{:.1})-({:.1},{:.1})",
                        placement.asset_id,
                        edge_idx,
                        min_x,
                        min_z,
                        max_x,
                        max_z,
                    );
                    return Err(ExplicitServicePlacementRejection::RoadOverlap);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn parcel_geometry_overlaps_explicit_site(&self, geometry: &ParcelGeometry) -> bool {
        if self.building_sites.is_empty() {
            return false;
        }
        let candidate_indices = self.site_candidate_indices_for_bounds(
            geometry.aabb_min.x,
            geometry.aabb_min.y,
            geometry.aabb_max.x,
            geometry.aabb_max.y,
        );
        for building_idx in candidate_indices {
            let Some(building) = self.buildings.get(building_idx) else {
                continue;
            };
            if building.parcel_id != 0 || building.zone_type != ZoneType::None {
                continue;
            }
            let Some(site) = self.building_sites.get(building_idx) else {
                continue;
            };
            let (site_min_x, site_min_z, site_max_x, site_max_z) = site.lot_bounds();
            if site_min_x > geometry.aabb_max.x
                || site_max_x < geometry.aabb_min.x
                || site_min_z > geometry.aabb_max.y
                || site_max_z < geometry.aabb_min.y
            {
                continue;
            }
            if flat_support_footprints_overlap(
                &geometry.corners,
                &site.lot_footprint_world,
                EXPLICIT_SERVICE_SITE_OVERLAP_EPS_M,
            ) {
                return true;
            }
        }
        false
    }

    fn placement_overlaps_existing_flat_support(&self, placement: &ResolvedPlacement) -> bool {
        if self.building_sites.is_empty() {
            return false;
        }
        let (min_x, min_z, max_x, max_z) = self.placement_site_bounds(placement);
        let candidate_indices =
            self.neighbor_site_candidate_indices(placement, min_x, min_z, max_x, max_z);
        if candidate_indices.is_empty() {
            return false;
        }

        let footprint_world = self.placement_required_flat_support_footprint(placement);
        let (flat_min_x, flat_min_z, flat_max_x, flat_max_z) = polygon_bounds(&footprint_world);
        for building_idx in candidate_indices {
            let Some(site) = self.building_sites.get(building_idx) else {
                continue;
            };
            let (site_min_x, site_min_z, site_max_x, site_max_z) = site.bounds();
            if site_min_x > flat_max_x + BUILDING_SITE_NEIGHBOR_EPS_M
                || site_max_x < flat_min_x - BUILDING_SITE_NEIGHBOR_EPS_M
                || site_min_z > flat_max_z + BUILDING_SITE_NEIGHBOR_EPS_M
                || site_max_z < flat_min_z - BUILDING_SITE_NEIGHBOR_EPS_M
            {
                continue;
            }
            if flat_support_footprints_overlap(
                &footprint_world,
                &site.footprint_world,
                BUILDING_SITE_NEIGHBOR_EPS_M,
            ) {
                return true;
            }
        }
        false
    }

    fn validate_site_support_tie_in(
        &self,
        placement: &ResolvedPlacement,
        graph: &RegionGraph,
        road_surface: &RoadSurfaceSystem,
        terrain: &TerrainSystem,
    ) -> Result<(), DemandSpawnPlacementRejection> {
        let footprint_world = self.placement_required_flat_support_footprint(placement);
        if building_site_support_tie_in_is_valid(
            &footprint_world,
            placement.support_height_m,
            terrain,
            graph,
            road_surface,
        ) {
            return Ok(());
        }

        let (min_x, min_z, max_x, max_z) = self.placement_site_bounds(placement);
        debug_log!(
            "economy",
            "building placement rejected: asset={} parcel={} flat support footprint cannot tie into surrounding terrain/road within slope budget bounds=({:.1},{:.1})-({:.1},{:.1}) support_height_m={:.2}",
            placement.asset_id,
            placement.parcel_id,
            min_x,
            min_z,
            max_x,
            max_z,
            placement.support_height_m,
        );
        Err(DemandSpawnPlacementRejection::SiteSupportTieInInvalid)
    }

    fn resolve_site_support_height(
        &self,
        placement: &ResolvedPlacement,
        graph: &RegionGraph,
        road_surface: &RoadSurfaceSystem,
        terrain: &TerrainSystem,
    ) -> Result<f32, DemandSpawnPlacementRejection> {
        let driveway_candidates =
            self.driveway_connection_candidates(placement, graph, road_surface, terrain);
        if !driveway_candidates.is_empty() {
            if let Some(missing) = driveway_candidates
                .iter()
                .find(|candidate| candidate.height_m.is_none())
            {
                debug_log!(
                    "economy",
                    "building placement rejected: asset={} parcel={} driveway '{}' connection on claimed edge {} could not resolve a road surface",
                    placement.asset_id,
                    placement.parcel_id,
                    missing.name,
                    placement.edge_idx,
                );
                return Err(DemandSpawnPlacementRejection::DrivewayRoadSurfaceMissing);
            }

            let primary = &driveway_candidates[0];
            let Some(primary_height) = primary.height_m else {
                return Err(DemandSpawnPlacementRejection::DrivewayRoadSurfaceMissing);
            };
            for candidate in driveway_candidates.iter().skip(1) {
                let Some(height_m) = candidate.height_m else {
                    return Err(DemandSpawnPlacementRejection::DrivewayRoadSurfaceMissing);
                };
                if (height_m - primary_height).abs() > BUILDING_SITE_DRIVEWAY_HEIGHT_CONFLICT_EPS_M
                {
                    debug_log!(
                        "economy",
                        "building placement rejected: asset={} parcel={} driveway '{}' height {:.2} conflicts with '{}' height {:.2}",
                        placement.asset_id,
                        placement.parcel_id,
                        candidate.name,
                        height_m,
                        primary.name,
                        primary_height,
                    );
                    return Err(DemandSpawnPlacementRejection::DrivewayHeightConflict);
                }
            }
            return self.validate_neighbor_site_height(placement, primary_height);
        }

        if self.driveway_anchor_count(placement) > 0 {
            debug_log!(
                "economy",
                "building placement rejected: asset={} parcel={} has driveway anchors but no valid claimed-edge driveway connection",
                placement.asset_id,
                placement.parcel_id,
            );
            return Err(DemandSpawnPlacementRejection::DrivewayConnectionMissing);
        }

        if let Some(frontage_height) =
            self.frontage_connection_height(placement, graph, road_surface, terrain)
        {
            return self.validate_neighbor_site_height(placement, frontage_height);
        }

        if self.asset_allows_source_terrain_site_fallback(&placement.asset_id) {
            let terrain_height = terrain
                .sample_height_world(placement.center_2d.x, placement.center_2d.y)
                * crate::config::HEIGHT_SCALE;
            return self.validate_neighbor_site_height(placement, terrain_height);
        }

        debug_log!(
            "economy",
            "building placement rejected: asset={} parcel={} frontage connection on claimed edge {} could not resolve a road surface",
            placement.asset_id,
            placement.parcel_id,
            placement.edge_idx,
        );
        Err(DemandSpawnPlacementRejection::FrontageRoadSurfaceMissing)
    }

    fn driveway_anchor_count(&self, placement: &ResolvedPlacement) -> usize {
        self.registry
            .get(&placement.asset_id)
            .map(|entry| {
                entry
                    .manifest
                    .anchors
                    .iter()
                    .filter(|anchor| anchor.anchor_type == AnchorType::Driveway)
                    .count()
            })
            .unwrap_or(0)
    }

    fn driveway_connection_candidates(
        &self,
        placement: &ResolvedPlacement,
        graph: &RegionGraph,
        road_surface: &RoadSurfaceSystem,
        terrain: &TerrainSystem,
    ) -> Vec<DrivewayConnectionCandidate> {
        let Some(entry) = self.registry.get(&placement.asset_id) else {
            return Vec::new();
        };
        let Some(edge) = graph.get_edge(placement.edge_idx) else {
            return Vec::new();
        };
        if edge.deleted || edge.physical_length <= 1e-6 || edge.physical_geometry.len() < 2 {
            return Vec::new();
        };
        let frontage_forward = entry.manifest.building_frontage_forward();
        let (basis_x, basis_z) =
            crate::simulation::buildings::allocator::entrance::building_local_xz_basis(
                placement.facing_dir,
                frontage_forward,
            );
        let frontage_center = placement.center_2d
            + placement.facing_dir * (placement.depth_cells as f32 * placement.zone_cell_m * 0.5);
        let inward_dir = if placement.facing_dir.length_squared() > 1e-12 {
            -placement.facing_dir.normalized()
        } else {
            Vector2::new(0.0, 1.0)
        };
        let mut driveways = entry
            .manifest
            .anchors
            .iter()
            .enumerate()
            .filter_map(|(authored_order, anchor)| {
                if anchor.anchor_type != AnchorType::Driveway {
                    return None;
                }
                let pos = placement.center_2d
                    + basis_x * anchor.position[0]
                    + basis_z * anchor.position[2];
                let edge_s_m = Self::project_point_to_polyline_s(&edge.physical_geometry, pos);
                let edge_t = edge_s_m / edge.physical_length;
                let connection_pos = Self::claimed_road_side_connection_pos(
                    graph,
                    placement.edge_idx,
                    placement.side,
                    edge_t,
                )?;
                let height_m = road_surface.sample_visible_surface_height(
                    graph,
                    terrain,
                    connection_pos.x,
                    connection_pos.y,
                );
                Some(DrivewayConnectionCandidate {
                    name: anchor.name.clone(),
                    authored_order,
                    distance_to_frontage_m: (pos - frontage_center).dot(inward_dir).abs(),
                    height_m,
                })
            })
            .collect::<Vec<_>>();
        sort_driveway_connection_candidates(&mut driveways);
        driveways
    }

    fn frontage_connection_height(
        &self,
        placement: &ResolvedPlacement,
        graph: &RegionGraph,
        road_surface: &RoadSurfaceSystem,
        terrain: &TerrainSystem,
    ) -> Option<f32> {
        let pos = Self::claimed_road_side_connection_pos(
            graph,
            placement.edge_idx,
            placement.side,
            placement.frontage_t,
        )?;
        road_surface.sample_visible_surface_height(graph, terrain, pos.x, pos.y)
    }

    fn claimed_road_side_connection_pos(
        graph: &RegionGraph,
        edge_idx: usize,
        side: i8,
        edge_t: f32,
    ) -> Option<Vector2> {
        let edge = graph.get_edge(edge_idx)?;
        if edge.deleted || edge.physical_length <= 1e-6 || edge.physical_geometry.len() < 2 {
            return None;
        }
        let center = Self::sample_pos_on_edge(graph, edge_idx, edge_t);
        let tangent = Self::sample_tangent_on_edge(graph, edge_idx, edge_t);
        if tangent.length_squared() <= 1e-12 {
            return None;
        }
        let normal = Vector2::new(tangent.y, -tangent.x) * side as f32;
        Some(center + normal * road_connection_lateral_offset_m(edge))
    }

    fn asset_allows_source_terrain_site_fallback(&self, asset_id: &str) -> bool {
        self.registry
            .get(asset_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .is_some_and(|building| building.placement_mode == PlacementMode::Explicit)
    }

    fn validate_neighbor_site_height(
        &self,
        placement: &ResolvedPlacement,
        support_height_m: f32,
    ) -> Result<f32, DemandSpawnPlacementRejection> {
        if self.building_sites.is_empty() {
            return Ok(support_height_m);
        }
        let footprint_world = self.placement_required_flat_support_footprint(placement);
        let (min_x, min_z, max_x, max_z) = polygon_bounds(&footprint_world);
        let candidate_indices =
            self.neighbor_site_candidate_indices(placement, min_x, min_z, max_x, max_z);
        for building_idx in candidate_indices {
            let Some(site) = self.building_sites.get(building_idx) else {
                continue;
            };
            let (site_min_x, site_min_z, site_max_x, site_max_z) = site.bounds();
            if site_min_x > max_x + BUILDING_SITE_NEIGHBOR_EPS_M
                || site_max_x < min_x - BUILDING_SITE_NEIGHBOR_EPS_M
                || site_min_z > max_z + BUILDING_SITE_NEIGHBOR_EPS_M
                || site_max_z < min_z - BUILDING_SITE_NEIGHBOR_EPS_M
            {
                continue;
            }
            if (site.support_height_m - support_height_m).abs()
                > BUILDING_SITE_NEIGHBOR_HEIGHT_EPS_M
                && flat_support_footprints_overlap(
                    &footprint_world,
                    &site.footprint_world,
                    BUILDING_SITE_NEIGHBOR_EPS_M,
                )
            {
                debug_log!(
                    "economy",
                    "building placement rejected: asset={} parcel={} site height {:.2} conflicts with overlapping neighboring building {} height {:.2}",
                    placement.asset_id,
                    placement.parcel_id,
                    support_height_m,
                    building_idx,
                    site.support_height_m,
                );
                return Err(DemandSpawnPlacementRejection::NeighborSiteHeightConflict);
            }
        }
        Ok(support_height_m)
    }

    fn neighbor_site_candidate_indices(
        &self,
        placement: &ResolvedPlacement,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Vec<usize> {
        if self.dirty_index
            || self.building_sites.len() != self.buildings.len()
            || self.building_chunks.is_empty()
        {
            return (0..self.building_sites.len()).collect();
        }

        let margin_m = self.max_lot_radius_cells * placement.zone_cell_m
            + BUILDING_SITE_NEIGHBOR_EPS_M.max(0.0);
        let chunk_size = RegionGraph::CHUNK_SIZE;
        let min_chunk_x = ((min_x - margin_m) / chunk_size).floor() as i32;
        let max_chunk_x = ((max_x + margin_m) / chunk_size).floor() as i32;
        let min_chunk_z = ((min_z - margin_m) / chunk_size).floor() as i32;
        let max_chunk_z = ((max_z + margin_m) / chunk_size).floor() as i32;

        let mut candidates = Vec::new();
        for chunk_x in min_chunk_x..=max_chunk_x {
            for chunk_z in min_chunk_z..=max_chunk_z {
                let Some(indices) = self.building_chunks.get(&(chunk_x, chunk_z)) else {
                    continue;
                };
                candidates.extend(indices.iter().copied());
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        candidates.retain(|&idx| idx < self.building_sites.len());
        candidates
    }

    fn placement_site_corners(&self, placement: &ResolvedPlacement) -> [Vector2; 4] {
        let frontage_forward = self
            .registry
            .get(&placement.asset_id)
            .map(|entry| entry.manifest.building_frontage_forward())
            .unwrap_or([0.0, 0.0, 1.0]);
        let (basis_x, basis_z) =
            crate::simulation::buildings::allocator::entrance::building_local_xz_basis(
                placement.facing_dir,
                frontage_forward,
            );
        let half_width = placement.width_cells as f32 * placement.zone_cell_m * 0.5;
        let half_depth = placement.depth_cells as f32 * placement.zone_cell_m * 0.5;
        [
            placement.center_2d + basis_x * -half_width + basis_z * -half_depth,
            placement.center_2d + basis_x * -half_width + basis_z * half_depth,
            placement.center_2d + basis_x * half_width + basis_z * half_depth,
            placement.center_2d + basis_x * half_width + basis_z * -half_depth,
        ]
    }

    fn placement_site_bounds(&self, placement: &ResolvedPlacement) -> (f32, f32, f32, f32) {
        let points = self.placement_site_corners(placement);
        let mut min_x = f32::INFINITY;
        let mut min_z = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_z = f32::NEG_INFINITY;
        for point in points {
            min_x = min_x.min(point.x);
            min_z = min_z.min(point.y);
            max_x = max_x.max(point.x);
            max_z = max_z.max(point.y);
        }
        (min_x, min_z, max_x, max_z)
    }

    fn placement_required_flat_support_footprint(
        &self,
        placement: &ResolvedPlacement,
    ) -> Vec<Vector2> {
        self.required_flat_support_footprint_world(
            &placement.asset_id,
            placement.center_2d,
            placement.facing_dir,
            placement.width_cells,
            placement.depth_cells,
            placement.zone_cell_m,
        )
    }

    fn place_building_instance(
        &mut self,
        placement: ResolvedPlacement,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> usize {
        let economy_binding = resolve_building_economy_profile_binding_with_catalog(
            &self.registry,
            catalog,
            &placement.asset_id,
        );
        let resource_count = catalog.resource_count();

        // Seed starting inventory for output ports when the profile specifies it.
        // This lets stores open with stock already on shelves before the first freight
        // delivery arrives, which is critical during the startup phase.
        let mut resource_inventory = vec![0.0f32; resource_count];
        if let Some(profile) = catalog.profile_by_runtime_id(economy_binding.runtime_id) {
            if profile.starting_inventory_days > 0.0 {
                for output in &profile.outputs {
                    let cap = profile.output_buffer_capacity_units_for(output);
                    let seed = (output.units_per_day * profile.starting_inventory_days).min(cap);
                    // resource_inventory is 0-indexed; runtime_id is 1-based.
                    let slot = output.resource_runtime_id as usize;
                    if slot > 0 && slot <= resource_count {
                        resource_inventory[slot - 1] = seed;
                    }
                }
            }
        }

        // Seed startup operating capital so commercial and industrial buildings can pay
        // workers for STARTUP_RUNWAY_DAYS before first revenue arrives, AND cover the cost
        // of the first OWA input import (which fires immediately when local supply is absent).
        // Computed from profile data so it scales with the building.
        const STARTUP_RUNWAY_DAYS: f32 = 7.0;
        const STARTUP_MIN_BUDGET: f32 = 500.0;
        let startup_budget = match placement.zone_type {
            ZoneType::None
                if !economy_binding.economy_broken
                    && catalog
                        .profile_by_runtime_id(economy_binding.runtime_id)
                        .is_some_and(|profile| {
                            matches!(
                                profile.kind,
                                EconomyProfileRuntimeKind::Extractor
                                    | EconomyProfileRuntimeKind::FieldProducer
                            )
                        }) =>
            {
                let profile = catalog
                    .profile_by_runtime_id(economy_binding.runtime_id)
                    .expect("explicit industry area profile checked above");
                let daily_wage = profile.average_daily_wage();
                (profile.worker_capacity as f32 * daily_wage * STARTUP_RUNWAY_DAYS)
                    .max(STARTUP_MIN_BUDGET)
            }
            ZoneType::Commercial | ZoneType::Industrial => {
                if economy_binding.economy_broken {
                    0.0
                } else {
                    let profile = catalog
                        .profile_by_runtime_id(economy_binding.runtime_id)
                        .unwrap_or_else(|| {
                            panic!(
                                "asset '{}' references missing runtime economy profile id {}",
                                placement.asset_id, economy_binding.runtime_id
                            )
                        });
                    let daily_wage = profile.average_daily_wage();
                    let wage_runway =
                        profile.worker_capacity as f32 * daily_wage * STARTUP_RUNWAY_DAYS;

                    // Add expected cost of the first full OWA input import so the building can
                    // absorb it without going into distress on its opening day.
                    let owa_import_multiplier = tuning.owa_import_price_multiplier;
                    let first_import_base_cost = profile
                        .inputs
                        .iter()
                        .map(|port| {
                            let unit_price = catalog
                                .unit_price_for_resource(port.resource_runtime_id)
                                .unwrap_or_else(|| {
                                    let resource_id = catalog
                                        .resource_id_for_runtime_id(port.resource_runtime_id)
                                        .unwrap_or("<unknown>");
                                    panic!(
                                        "resource '{resource_id}' used by profile '{}' has no catalog price",
                                        profile.id
                                    )
                                });
                            profile.inventory_target_units_for(port)
                                * unit_price
                                * owa_import_multiplier
                        })
                        .sum::<f32>();
                    (wage_runway + first_import_base_cost).max(STARTUP_MIN_BUDGET)
                }
            }
            _ => 0.0,
        };

        let construction_duration_hours =
            construction_duration_hours(placement.zone_type, placement.initial_level, tuning);
        let zone_cell_m = placement.zone_cell_m;
        let profile_kind = catalog
            .profile_by_runtime_id(economy_binding.runtime_id)
            .map(|profile| profile.kind);
        let work_area_scale = initial_work_area_scale(placement.zone_type, profile_kind);

        self.buildings.push(Building {
            zone_profile_runtime_id: placement.zone_profile_runtime_id,
            parcel_id: placement.parcel_id,
            zone_type: placement.zone_type,
            facing_dir: placement.facing_dir,
            frontage_t: placement.frontage_t,
            side_offset: placement.edge_width * 0.5 + crate::config::SIDEWALK_WIDTH,
            center_x: placement.center_2d.x,
            center_y: placement.center_2d.y,
            support_height_m: placement.support_height_m,
            edge_idx: placement.edge_idx,
            side: placement.side,
            cell_x: placement.cell_x,
            cell_y: 0,
            width_cells: placement.width_cells as u16,
            depth_cells: placement.depth_cells as u16,
            occupancy: 0,
            worker_count: 0,
            service_funding_override: -1.0,
            asset_id: placement.asset_id,
            level: placement.initial_level,
            construction_total_hours: construction_duration_hours,
            construction_remaining_hours: construction_duration_hours,
            broken: false,
            economy_profile_runtime_id: economy_binding.runtime_id,
            economy_broken: economy_binding.economy_broken,
            resource_inventory,
            revenue: 0.0,
            operating_budget: startup_budget,
            profit_tax_budget_baseline: startup_budget,
            last_day_profit: 0.0,
            shipment_cooldown_hours: 0,
            daily_owa_input_value: 0.0,
            daily_local_input_value: 0.0,
            daily_city_funded_input_cost: 0.0,
            daily_household_sales_value: 0.0,
            daily_power_service_units: 0.0,
            daily_power_served_units: 0.0,
            recent_power_service_units: 0.0,
            recent_power_served_units: 0.0,
            recent_household_sales_value: 0.0,
            commercial_activity_floor_scale: 0.0,
            work_area_scale,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
            is_deserted: false,
            budget_distress: false,
        });
        let building_idx = self.buildings.len() - 1;
        self.push_building_site_client(building_idx, zone_cell_m);
        building_idx
    }
}

fn flat_support_footprints_overlap(left: &[Vector2], right: &[Vector2], eps_m: f32) -> bool {
    if left.len() < 3 || right.len() < 3 {
        return false;
    }
    !has_separating_axis(left, left, right, eps_m)
        && !has_separating_axis(right, left, right, eps_m)
}

fn has_separating_axis(
    axis_source: &[Vector2],
    left: &[Vector2],
    right: &[Vector2],
    eps_m: f32,
) -> bool {
    for idx in 0..axis_source.len() {
        let start = axis_source[idx];
        let end = axis_source[(idx + 1) % axis_source.len()];
        let edge = end - start;
        let length = edge.length();
        if length <= f32::EPSILON {
            continue;
        }
        let axis = Vector2::new(-edge.y, edge.x) / length;
        let (left_min, left_max) = project_polygon(left, axis);
        let (right_min, right_max) = project_polygon(right, axis);
        if left_max <= right_min + eps_m || right_max <= left_min + eps_m {
            return true;
        }
    }
    false
}

fn project_polygon(points: &[Vector2], axis: Vector2) -> (f32, f32) {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for point in points {
        let projected = point.dot(axis);
        min = min.min(projected);
        max = max.max(projected);
    }
    (min, max)
}

fn polygon_bounds(points: &[Vector2]) -> (f32, f32, f32, f32) {
    let mut min_x = f32::INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for point in points {
        min_x = min_x.min(point.x);
        min_z = min_z.min(point.y);
        max_x = max_x.max(point.x);
        max_z = max_z.max(point.y);
    }
    (min_x, min_z, max_x, max_z)
}

fn construction_duration_hours(
    zone_type: ZoneType,
    level: u8,
    tuning: &RuntimeEconomyTuning,
) -> u16 {
    let levels = match zone_type {
        ZoneType::Residential => &tuning.construction.residential_hours_by_level,
        ZoneType::Commercial => &tuning.construction.commercial_hours_by_level,
        ZoneType::Industrial => &tuning.construction.industrial_hours_by_level,
        ZoneType::None | ZoneType::Office | ZoneType::Mixed => return 0,
    };
    let idx = usize::from(level.saturating_sub(1)).min(levels.len().saturating_sub(1));
    levels.get(idx).copied().unwrap_or(0)
}

const BUILDING_SITE_DRIVEWAY_HEIGHT_CONFLICT_EPS_M: f32 = 0.35;
const BUILDING_SITE_NEIGHBOR_HEIGHT_EPS_M: f32 = 0.10;
const BUILDING_SITE_NEIGHBOR_EPS_M: f32 = 0.05;
const BUILDING_SITE_ROAD_SAMPLE_INSET_M: f32 = 0.05;

struct DrivewayConnectionCandidate {
    name: String,
    authored_order: usize,
    distance_to_frontage_m: f32,
    height_m: Option<f32>,
}

/// World-space preview of one explicit service building footprint.
pub(crate) struct ExplicitServicePlacementPreview {
    /// Ground-plane footprint corners in metres, ordered around the lot perimeter.
    pub(crate) corners: [Vector2; 4],
    /// World-space centre of the snapped candidate footprint.
    pub(crate) center_2d: Vector2,
    /// Flat support height selected for the building site in rendered metres.
    pub(crate) support_height_m: f32,
    /// Unit vector pointing from the candidate frontage toward the road.
    pub(crate) facing_dir: Vector2,
    /// True when the preview can be committed without changing its current placement.
    pub(crate) valid: bool,
    /// Rejection reason for snapable previews that cannot be committed.
    pub(crate) rejection: Option<ExplicitServicePlacementRejection>,
}

fn expected_utility_service_for_class(service_class: &str) -> Option<&'static str> {
    match service_class.trim() {
        "power" => Some("power"),
        "water" => Some("water"),
        "waste" => Some("sewage"),
        _ => None,
    }
}

fn explicit_rejection_from_site_rejection(
    rejection: DemandSpawnPlacementRejection,
) -> ExplicitServicePlacementRejection {
    match rejection {
        DemandSpawnPlacementRejection::DrivewayRoadSurfaceMissing => {
            ExplicitServicePlacementRejection::DrivewayRoadSurfaceMissing
        }
        DemandSpawnPlacementRejection::DrivewayHeightConflict => {
            ExplicitServicePlacementRejection::DrivewayHeightConflict
        }
        DemandSpawnPlacementRejection::DrivewayConnectionMissing => {
            ExplicitServicePlacementRejection::DrivewayConnectionMissing
        }
        DemandSpawnPlacementRejection::FrontageRoadSurfaceMissing => {
            ExplicitServicePlacementRejection::FrontageRoadSurfaceMissing
        }
        DemandSpawnPlacementRejection::NeighborSiteHeightConflict => {
            ExplicitServicePlacementRejection::NeighborSiteHeightConflict
        }
        DemandSpawnPlacementRejection::SiteSupportTieInInvalid => {
            ExplicitServicePlacementRejection::SiteSupportTieInInvalid
        }
        DemandSpawnPlacementRejection::AssetUnavailable
        | DemandSpawnPlacementRejection::ParcelUnavailable
        | DemandSpawnPlacementRejection::SlotUnavailable => {
            ExplicitServicePlacementRejection::RoadFrontageUnavailable
        }
    }
}

fn sort_driveway_connection_candidates(candidates: &mut [DrivewayConnectionCandidate]) {
    candidates.sort_by(|left, right| {
        left.distance_to_frontage_m
            .total_cmp(&right.distance_to_frontage_m)
            .then(left.authored_order.cmp(&right.authored_order))
    });
}

fn road_connection_lateral_offset_m(edge: &crate::simulation::network::graph::Edge) -> f32 {
    let sidewalk_m = if edge.primary_type == TransitType::Foot
        || (edge.allowed_types & TransitFlags::FOOT) == 0
    {
        0.0
    } else {
        SIDEWALK_WIDTH
    };
    (edge.width * 0.5 + sidewalk_m - BUILDING_SITE_ROAD_SAMPLE_INSET_M).max(0.0)
}

fn road_collision_half_width_m(edge: &crate::simulation::network::graph::Edge) -> f32 {
    let sidewalk_m = if edge.primary_type == TransitType::Foot
        || (edge.allowed_types & TransitFlags::FOOT) == 0
    {
        0.0
    } else {
        SIDEWALK_WIDTH
    };
    (edge.width * 0.5 + sidewalk_m).max(0.0)
}

fn stable_strip_family_hash(profile_runtime_id: u16, parcel_id: u64, family_key: &str) -> u64 {
    let mut hasher = StableHasher::new();
    hasher.write_u16(profile_runtime_id);
    hasher.write_u64(parcel_id);
    hasher.write_str(family_key);
    hasher.finish()
}

fn stable_site_variant_hash(
    profile_runtime_id: u16,
    parcel_id: u64,
    qualified_asset_id: &str,
) -> u64 {
    let mut hasher = StableHasher::new();
    hasher.write_u16(profile_runtime_id);
    hasher.write_u64(parcel_id);
    hasher.write_str(qualified_asset_id);
    hasher.finish()
}

struct StableHasher {
    state: u64,
}

impl StableHasher {
    fn new() -> Self {
        Self {
            state: 0xcbf29ce484222325,
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state ^= byte as u64;
            self.state = self.state.wrapping_mul(0x100000001b3);
        }
        self.state ^= 0xff;
        self.state = self.state.wrapping_mul(0x100000001b3);
    }

    fn write_u16(&mut self, value: u16) {
        self.write_bytes(&value.to_le_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.write_bytes(&value.to_le_bytes());
    }

    fn write_str(&mut self, value: &str) {
        self.write_bytes(value.as_bytes());
    }

    fn finish(self) -> u64 {
        self.state
    }
}

struct AssetPlacementParams {
    zone_type: ZoneType,
    density: String,
    tags: Vec<String>,
    width_cells: usize,
    depth_cells: usize,
    initial_level: u8,
}

struct SpawnAssetCandidate {
    family_key: String,
    qualified_id: String,
    params: AssetPlacementParams,
}

struct SpawnProfileAssetCandidates {
    candidates: Vec<SpawnAssetCandidate>,
}

struct ResolvedPlacement {
    asset_id: String,
    zone_profile_runtime_id: u16,
    zone_type: ZoneType,
    initial_level: u8,
    parcel_id: u64,
    edge_idx: usize,
    side: i8,
    cell_x: usize,
    width_cells: usize,
    depth_cells: usize,
    zone_cell_m: f32,
    center_2d: Vector2,
    support_height_m: f32,
    facing_dir: Vector2,
    frontage_t: f32,
    edge_width: f32,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct DemandSpawnCandidateSortKey {
    edge_idx: usize,
    side_order: u8,
    cell_x: usize,
    width_cells: usize,
    depth_cells: usize,
    zone_profile_runtime_id: u16,
    parcel_id: u64,
}

impl DemandSpawnCandidateSortKey {
    fn from_resolved(placement: &ResolvedPlacement) -> Self {
        Self {
            edge_idx: placement.edge_idx,
            side_order: spawn_side_order(placement.side),
            cell_x: placement.cell_x,
            width_cells: placement.width_cells,
            depth_cells: placement.depth_cells,
            zone_profile_runtime_id: placement.zone_profile_runtime_id,
            parcel_id: placement.parcel_id,
        }
    }
}

#[derive(Default)]
struct DemandSpawnCandidateSortBuckets {
    residential: Vec<(DemandSpawnCandidateSortKey, DemandSpawnCandidate)>,
    commercial: Vec<(DemandSpawnCandidateSortKey, DemandSpawnCandidate)>,
    industrial: Vec<(DemandSpawnCandidateSortKey, DemandSpawnCandidate)>,
}

impl DemandSpawnCandidateSortBuckets {
    fn push_zone_type(
        &mut self,
        zone_type: ZoneType,
        sort_key: DemandSpawnCandidateSortKey,
        candidate: DemandSpawnCandidate,
    ) {
        match zone_type {
            ZoneType::Residential => self.residential.push((sort_key, candidate)),
            ZoneType::Commercial => self.commercial.push((sort_key, candidate)),
            ZoneType::Industrial => self.industrial.push((sort_key, candidate)),
            _ => {}
        }
    }

    fn extend(&mut self, other: Self) {
        self.residential.extend(other.residential);
        self.commercial.extend(other.commercial);
        self.industrial.extend(other.industrial);
    }

    fn finish(mut self) -> DemandSpawnCandidatesByUse {
        sort_demand_spawn_candidates(&mut self.residential);
        sort_demand_spawn_candidates(&mut self.commercial);
        sort_demand_spawn_candidates(&mut self.industrial);

        let mut candidates = DemandSpawnCandidatesByUse::default();
        for (_, candidate) in self.residential {
            candidates.push_zone_type(ZoneType::Residential, candidate);
        }
        for (_, candidate) in self.commercial {
            candidates.push_zone_type(ZoneType::Commercial, candidate);
        }
        for (_, candidate) in self.industrial {
            candidates.push_zone_type(ZoneType::Industrial, candidate);
        }
        candidates
    }
}

fn sort_demand_spawn_candidates(
    candidates: &mut [(DemandSpawnCandidateSortKey, DemandSpawnCandidate)],
) {
    candidates.sort_unstable_by(|left, right| left.0.cmp(&right.0));
}

fn spawn_side_order(side: i8) -> u8 {
    if side > 0 { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square_footprint(min_x: f32, min_z: f32, max_x: f32, max_z: f32) -> Vec<Vector2> {
        vec![
            Vector2::new(min_x, min_z),
            Vector2::new(min_x, max_z),
            Vector2::new(max_x, max_z),
            Vector2::new(max_x, min_z),
        ]
    }

    fn spawn_candidate(parcel_id: u64) -> DemandSpawnCandidate {
        DemandSpawnCandidate {
            action: DemandSpawnAction {
                parcel_id,
                asset_id: "test:asset".to_owned(),
            },
            density: "low".to_owned(),
        }
    }

    #[test]
    fn flat_support_footprints_do_not_overlap_when_edges_touch() {
        let left = square_footprint(0.0, 0.0, 10.0, 10.0);
        let right = square_footprint(10.0, 0.0, 20.0, 10.0);

        assert!(!flat_support_footprints_overlap(
            &left,
            &right,
            BUILDING_SITE_NEIGHBOR_EPS_M
        ));
    }

    #[test]
    fn flat_support_footprints_overlap_with_positive_area() {
        let left = square_footprint(0.0, 0.0, 10.0, 10.0);
        let right = square_footprint(9.0, 0.0, 19.0, 10.0);

        assert!(flat_support_footprints_overlap(
            &left,
            &right,
            BUILDING_SITE_NEIGHBOR_EPS_M
        ));
    }

    #[test]
    fn zoned_spawn_prefilter_allows_touching_flat_supports() {
        let mut allocator = BuildingAllocator::new();
        allocator
            .building_sites
            .push(super::super::site::BuildingSiteClient {
                footprint_world: square_footprint(0.0, 0.0, 10.0, 10.0),
                lot_footprint_world: [
                    Vector2::new(0.0, 0.0),
                    Vector2::new(0.0, 10.0),
                    Vector2::new(10.0, 10.0),
                    Vector2::new(10.0, 0.0),
                ],
                support_height_m: 12.0,
                surfaces: Vec::new(),
            });
        let touching = ResolvedPlacement {
            asset_id: "test:asset".to_owned(),
            zone_profile_runtime_id: 1,
            zone_type: ZoneType::Residential,
            initial_level: 1,
            parcel_id: 2,
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            width_cells: 1,
            depth_cells: 1,
            zone_cell_m: 10.0,
            center_2d: Vector2::new(15.0, 5.0),
            support_height_m: 14.0,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            edge_width: 8.0,
        };
        let overlapping = ResolvedPlacement {
            asset_id: "test:asset".to_owned(),
            zone_profile_runtime_id: 1,
            zone_type: ZoneType::Residential,
            initial_level: 1,
            parcel_id: 3,
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            width_cells: 1,
            depth_cells: 1,
            zone_cell_m: 10.0,
            center_2d: Vector2::new(14.0, 5.0),
            support_height_m: 14.0,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            edge_width: 8.0,
        };

        assert!(!allocator.placement_overlaps_existing_flat_support(&touching));
        assert!(allocator.placement_overlaps_existing_flat_support(&overlapping));
    }

    #[test]
    fn demand_spawn_candidates_sort_by_build_site_order() {
        let mut candidates = vec![
            (
                DemandSpawnCandidateSortKey {
                    edge_idx: 3,
                    side_order: 0,
                    cell_x: 0,
                    width_cells: 2,
                    depth_cells: 2,
                    zone_profile_runtime_id: 1,
                    parcel_id: 10,
                },
                spawn_candidate(10),
            ),
            (
                DemandSpawnCandidateSortKey {
                    edge_idx: 2,
                    side_order: 1,
                    cell_x: 0,
                    width_cells: 2,
                    depth_cells: 2,
                    zone_profile_runtime_id: 1,
                    parcel_id: 20,
                },
                spawn_candidate(20),
            ),
            (
                DemandSpawnCandidateSortKey {
                    edge_idx: 2,
                    side_order: 0,
                    cell_x: 1,
                    width_cells: 2,
                    depth_cells: 2,
                    zone_profile_runtime_id: 1,
                    parcel_id: 30,
                },
                spawn_candidate(30),
            ),
            (
                DemandSpawnCandidateSortKey {
                    edge_idx: 2,
                    side_order: 0,
                    cell_x: 0,
                    width_cells: 2,
                    depth_cells: 2,
                    zone_profile_runtime_id: 1,
                    parcel_id: 40,
                },
                spawn_candidate(40),
            ),
        ];

        sort_demand_spawn_candidates(&mut candidates);
        let parcel_ids: Vec<_> = candidates
            .iter()
            .map(|(_, candidate)| candidate.action.parcel_id)
            .collect();

        assert_eq!(parcel_ids, vec![40, 30, 20, 10]);
    }

    #[test]
    fn driveway_candidates_sort_by_frontage_distance_then_authored_order() {
        let mut candidates = vec![
            DrivewayConnectionCandidate {
                name: "z_far".to_owned(),
                authored_order: 1,
                distance_to_frontage_m: 4.0,
                height_m: Some(0.0),
            },
            DrivewayConnectionCandidate {
                name: "b_near_second".to_owned(),
                authored_order: 3,
                distance_to_frontage_m: 1.0,
                height_m: Some(0.0),
            },
            DrivewayConnectionCandidate {
                name: "a_near_first".to_owned(),
                authored_order: 2,
                distance_to_frontage_m: 1.0,
                height_m: Some(0.0),
            },
        ];

        sort_driveway_connection_candidates(&mut candidates);

        let names = candidates
            .iter()
            .map(|candidate| candidate.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["a_near_first", "b_near_second", "z_far"]);
    }
}
