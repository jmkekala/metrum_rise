// SPDX-License-Identifier: GPL-2.0-only

//! Field-production sites owned by explicit agricultural buildings.
//!
//! Farms are renewable producers: a placed farm owns one player-drawn field
//! polygon, and the polygon area scales the authored profile yield. Unlike
//! extraction sites, fields do not snapshot or deplete a map-authored resource
//! deposit.

mod clearance;
pub(crate) use clearance::{FieldClearanceIndex, PolygonFootprint};

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntimeKind, RuntimeEconomyCatalog, load_runtime_economy_catalog,
};
use crate::simulation::economy::households::{
    OPERATIONAL_HOURS_PER_DAY, building_operation_factors, consume_hourly_production_inputs,
    scaled_output_buffer_capacity_units_for_building,
};
use crate::simulation::extraction::{validate_player_polygon, validate_polygon_near_building};
use crate::simulation::network::surface::RoadSurfaceSystem;
use crate::simulation::work_area::{
    explicit_work_area_scale, remove_work_area_owner, top_up_explicit_work_area_startup_budget,
};
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::Vector2;

/// Maximum accepted gap from the farm footprint to its field polygon.
pub(crate) const FIELD_POLYGON_LINK_DISTANCE_M: f32 = 10.0;

const MIN_FIELD_POLYGON_AREA_M2: f32 = 100.0;

/// One placed agricultural field and its owning farm building.
#[derive(Clone, Debug)]
pub(crate) struct FieldSite {
    /// Building index that owns this field polygon.
    pub(crate) building_idx: usize,
    /// Authored economy resource id grown by this field, such as `"grain"`.
    pub(crate) resource_id: String,
    /// Player-authored world-space polygon in metres, using `(x, z)`.
    pub(crate) polygon_world: Vec<Vector2>,
    /// Cached unsigned field polygon area in square metres.
    pub(crate) area_m2: f32,
}

/// Result returned after creating or replacing a field polygon.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FieldSiteSummary {
    /// Accepted field area in square metres.
    pub(crate) area_m2: f32,
}

/// Runtime agricultural field state for explicit field-producing buildings.
#[derive(Clone, Debug, Default)]
pub(crate) struct AgricultureSystem {
    // Unique, ascending owner indices support lookup and shared swap-remove repair.
    sites: Vec<FieldSite>,
    visual_revision: u64,
}

impl AgricultureSystem {
    /// Creates an empty agriculture system.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Restores field sites from persistence after validation.
    pub(crate) fn from_sites(mut sites: Vec<FieldSite>) -> Self {
        sites.sort_unstable_by_key(|site| site.building_idx);
        let visual_revision = u64::from(!sites.is_empty());
        Self {
            sites,
            visual_revision,
        }
    }

    /// Returns all committed field sites.
    pub(crate) fn sites(&self) -> &[FieldSite] {
        &self.sites
    }

    /// Monotonic visual-state revision for committed field overlay refreshes.
    pub(crate) fn visual_revision(&self) -> u64 {
        self.visual_revision
    }

    /// Clears all field sites.
    pub(crate) fn clear(&mut self) {
        if self.sites.is_empty() {
            return;
        }
        self.sites.clear();
        self.bump_visual_revision();
    }

    /// Returns the field site attached to one building, if present.
    pub(crate) fn site_for_building(&self, building_idx: usize) -> Option<&FieldSite> {
        self.sites
            .binary_search_by_key(&building_idx, |site| site.building_idx)
            .ok()
            .map(|idx| &self.sites[idx])
    }

    /// Removes sites for a swap-removed building and remaps the moved last building.
    pub(crate) fn remove_building_after_swap_remove(
        &mut self,
        removed_building_idx: usize,
        last_building_idx_before_remove: usize,
    ) {
        if remove_work_area_owner(
            &mut self.sites,
            removed_building_idx,
            last_building_idx_before_remove,
            |site| site.building_idx,
            |site, owner| site.building_idx = owner,
        ) {
            self.bump_visual_revision();
        }
    }

    /// Restores field sites affected by undoing one swap-remove building deletion.
    pub(crate) fn restore_sites_after_building_removal_undo(
        &mut self,
        restored_building_idx: usize,
        last_building_idx_before_remove: usize,
        restored_sites: Vec<FieldSite>,
        allocator: &mut BuildingAllocator,
    ) {
        allocator.field_clearance.set(restored_building_idx, &[]);
        allocator
            .field_clearance
            .set(last_building_idx_before_remove, &[]);
        for site in &restored_sites {
            allocator
                .field_clearance
                .set(site.building_idx, &site.polygon_world);
        }
        self.sites.retain(|site| {
            site.building_idx != restored_building_idx
                && site.building_idx != last_building_idx_before_remove
        });
        self.sites.extend(restored_sites);
        self.sites.sort_unstable_by_key(|site| site.building_idx);
        self.bump_visual_revision();
    }

    /// Commits or replaces the field polygon for one placed field-producing building.
    pub(crate) fn commit_site(
        &mut self,
        building_idx: usize,
        polygon_world: Vec<Vector2>,
        allocator: &mut BuildingAllocator,
        zoning: &ZoningSystem,
        roads: &RoadSurfaceSystem,
    ) -> Result<FieldSiteSummary, String> {
        let area_m2 = self.validate_site(building_idx, &polygon_world, allocator, zoning, roads)?;
        let building = allocator
            .buildings
            .get(building_idx)
            .ok_or_else(|| "field building does not exist".to_owned())?;
        let resource_id = allocator
            .registry
            .field_resource(&building.asset_id)
            .ok_or_else(|| "selected building is not a field producer".to_owned())?
            .to_owned();
        let position = self
            .sites
            .binary_search_by_key(&building_idx, |site| site.building_idx);
        let had_site = position.is_ok();
        allocator.field_clearance.set(building_idx, &polygon_world);
        let site = FieldSite {
            building_idx,
            resource_id,
            polygon_world,
            area_m2,
        };
        match position {
            Ok(idx) => self.sites[idx] = site,
            Err(idx) => self.sites.insert(idx, site),
        }
        self.bump_visual_revision();
        if let Some(building) = allocator.buildings.get_mut(building_idx) {
            let area_scale = explicit_work_area_scale(area_m2);
            building.set_work_area_scale(area_scale);
            if !had_site && let Ok(catalog) = load_runtime_economy_catalog() {
                top_up_explicit_work_area_startup_budget(building, catalog.as_ref(), area_scale);
            }
        }

        Ok(FieldSiteSummary { area_m2 })
    }

    /// Validates geometry, farm attachment and land-use clearance without changing the saved field.
    pub(crate) fn validate_site(
        &self,
        building_idx: usize,
        polygon_world: &[Vector2],
        allocator: &BuildingAllocator,
        zoning: &ZoningSystem,
        roads: &RoadSurfaceSystem,
    ) -> Result<f32, String> {
        let building = allocator
            .buildings
            .get(building_idx)
            .ok_or_else(|| "field building does not exist".to_owned())?;
        if !allocator
            .registry
            .is_field_producer_asset(&building.asset_id)
        {
            return Err("selected building is not a field producer".to_owned());
        }
        let area_m2 = validate_field_polygon_world(polygon_world)?;
        let site = allocator
            .building_sites
            .get(building_idx)
            .ok_or_else(|| "farm building site does not exist".to_owned())?;
        validate_polygon_near_building(
            &site.lot_footprint_world,
            polygon_world,
            "field",
            FIELD_POLYGON_LINK_DISTANCE_M,
        )?;
        let footprint = PolygonFootprint::new(polygon_world);
        if footprint.min.x < -zoning.config.width_m * 0.5
            || footprint.max.x > zoning.config.width_m * 0.5
            || footprint.min.y < -zoning.config.height_m * 0.5
            || footprint.max.y > zoning.config.height_m * 0.5
        {
            return Err("field must remain inside the world".to_owned());
        }
        if allocator
            .field_clearance
            .overlaps(&footprint, Some(building_idx))
        {
            return Err("field overlaps another field".to_owned());
        }
        if zoning.parcels.overlaps_polygon(&footprint) {
            return Err("field overlaps a zoning parcel".to_owned());
        }
        for idx in allocator.site_candidate_indices_for_bounds(
            footprint.min.x,
            footprint.min.y,
            footprint.max.x,
            footprint.max.y,
        ) {
            if let Some(site) = allocator.building_sites.get(idx)
                && footprint.overlaps(&PolygonFootprint::new(&site.footprint_world))
            {
                return Err("field overlaps a building site".to_owned());
            }
        }
        if footprint.overlaps_roads(roads) {
            return Err("field overlaps a road".to_owned());
        }
        Ok(area_m2)
    }

    /// Rebuilds cached building work-area scales from committed field sites.
    pub(crate) fn apply_work_area_scales(&self, allocator: &mut BuildingAllocator) {
        allocator.field_clearance.clear();
        for site in &self.sites {
            allocator
                .field_clearance
                .set(site.building_idx, &site.polygon_world);
            if let Some(building) = allocator.buildings.get_mut(site.building_idx) {
                building.set_work_area_scale(explicit_work_area_scale(site.area_m2));
            }
        }
    }

    /// Produces renewable field output from active farm sites for one operational hour.
    pub(crate) fn produce_hourly(
        &mut self,
        allocator: &mut BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
    ) {
        for site in &self.sites {
            let Some(resource_runtime_id) = catalog.resource_runtime_id_for_id(&site.resource_id)
            else {
                continue;
            };
            let Some(building) = allocator.buildings.get_mut(site.building_idx) else {
                continue;
            };
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
            {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            if profile.kind != EconomyProfileRuntimeKind::FieldProducer {
                continue;
            }
            let Some(output_port) = profile.output_port(resource_runtime_id) else {
                continue;
            };
            let factors = building_operation_factors(catalog, building, profile);
            if factors.throughput_factor <= 0.0 {
                continue;
            }
            let area_factor = explicit_work_area_scale(site.area_m2);
            if area_factor <= 0.0 {
                continue;
            }
            let hourly_units = output_port.units_per_day * area_factor / OPERATIONAL_HOURS_PER_DAY
                * factors.throughput_factor;
            if hourly_units <= 0.0 {
                continue;
            }
            let current = building.inventory_units(output_port.resource_runtime_id);
            let capacity =
                scaled_output_buffer_capacity_units_for_building(building, profile, output_port);
            let produced = hourly_units.min((capacity - current).max(0.0));
            if produced <= 0.0 {
                continue;
            }
            consume_hourly_production_inputs(
                building,
                profile,
                area_factor * factors.throughput_factor * (produced / hourly_units),
            );
            building.add_inventory_units(output_port.resource_runtime_id, produced);
        }
    }

    fn bump_visual_revision(&mut self) {
        self.visual_revision = self.visual_revision.wrapping_add(1);
    }
}

/// Validates field geometry and returns its unsigned world-space area.
pub(crate) fn validate_field_polygon_world(polygon_world: &[Vector2]) -> Result<f32, String> {
    validate_player_polygon(polygon_world, "field", MIN_FIELD_POLYGON_AREA_M2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_polygon_area_uses_world_square_metres() {
        let polygon = vec![
            Vector2::new(0.0, 0.0),
            Vector2::new(20.0, 0.0),
            Vector2::new(20.0, 10.0),
            Vector2::new(0.0, 10.0),
        ];

        assert!((validate_field_polygon_world(&polygon).unwrap() - 200.0).abs() <= f32::EPSILON);
    }
}
