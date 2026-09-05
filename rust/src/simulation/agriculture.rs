// SPDX-License-Identifier: GPL-2.0-only

//! Field-production sites owned by explicit agricultural buildings.
//!
//! Farms are renewable producers: a placed farm owns one player-drawn field
//! polygon, and the polygon area scales the authored profile yield. Unlike
//! extraction sites, fields do not snapshot or deplete a map-authored resource
//! deposit.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntimeKind, RuntimeEconomyCatalog, load_runtime_economy_catalog,
};
use crate::simulation::economy::households::{
    building_operation_factors, scaled_output_buffer_capacity_units_for_building,
};
use crate::simulation::extraction::{validate_player_polygon, validate_polygon_near_building};
use crate::simulation::work_area::{
    EXPLICIT_WORK_AREA_BASE_M2, top_up_explicit_work_area_startup_budget,
};
use godot::prelude::Vector2;

/// Maximum accepted gap from the farm footprint to its field polygon.
pub(crate) const FIELD_POLYGON_LINK_DISTANCE_M: f32 = 10.0;
/// Field area that receives exactly the authored daily output rate.
pub(crate) const FIELD_YIELD_BASE_AREA_M2: f32 = EXPLICIT_WORK_AREA_BASE_M2;

const OPERATIONAL_HOURS_PER_DAY: f32 = 24.0;
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
            .iter()
            .find(|site| site.building_idx == building_idx)
    }

    /// Removes sites for a swap-removed building and remaps the moved last building.
    pub(crate) fn remove_building_after_swap_remove(
        &mut self,
        removed_building_idx: usize,
        last_building_idx_before_remove: usize,
    ) {
        let old_len = self.sites.len();
        self.sites
            .retain(|site| site.building_idx != removed_building_idx);
        let mut changed = self.sites.len() != old_len;
        if removed_building_idx == last_building_idx_before_remove {
            if changed {
                self.bump_visual_revision();
            }
            return;
        }
        for site in &mut self.sites {
            if site.building_idx == last_building_idx_before_remove {
                site.building_idx = removed_building_idx;
                changed = true;
            }
        }
        if changed {
            self.bump_visual_revision();
        }
    }

    /// Restores field sites affected by undoing one swap-remove building deletion.
    pub(crate) fn restore_sites_after_building_removal_undo(
        &mut self,
        restored_building_idx: usize,
        last_building_idx_before_remove: usize,
        restored_sites: Vec<FieldSite>,
    ) {
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
        zone_cell_m: f32,
    ) -> Result<FieldSiteSummary, String> {
        let building = allocator
            .buildings
            .get(building_idx)
            .ok_or_else(|| "field building does not exist".to_owned())?;
        let resource_id = allocator
            .registry
            .field_resource(&building.asset_id)
            .ok_or_else(|| "selected building is not a field producer".to_owned())?
            .to_owned();
        let area_m2 = validate_field_polygon_world(&polygon_world)?;
        validate_polygon_near_building(
            building,
            &polygon_world,
            zone_cell_m,
            "field",
            FIELD_POLYGON_LINK_DISTANCE_M,
        )?;

        let had_site = self
            .sites
            .iter()
            .any(|site| site.building_idx == building_idx);
        let site = FieldSite {
            building_idx,
            resource_id,
            polygon_world,
            area_m2,
        };
        self.sites.retain(|site| site.building_idx != building_idx);
        self.sites.push(site);
        self.sites.sort_unstable_by_key(|site| site.building_idx);
        self.bump_visual_revision();
        if let Some(building) = allocator.buildings.get_mut(building_idx) {
            let area_scale = field_area_yield_factor(area_m2);
            building.set_work_area_scale(area_scale);
            if !had_site && let Ok(catalog) = load_runtime_economy_catalog() {
                top_up_explicit_work_area_startup_budget(building, catalog.as_ref(), area_scale);
            }
        }

        Ok(FieldSiteSummary { area_m2 })
    }

    /// Rebuilds cached building work-area scales from committed field sites.
    pub(crate) fn apply_work_area_scales(&self, allocator: &mut BuildingAllocator) {
        for site in &self.sites {
            if let Some(building) = allocator.buildings.get_mut(site.building_idx) {
                building.set_work_area_scale(field_area_yield_factor(site.area_m2));
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
            let area_factor = field_area_yield_factor(site.area_m2);
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
            building.add_inventory_units(output_port.resource_runtime_id, produced);
        }
    }

    fn bump_visual_revision(&mut self) {
        self.visual_revision = self.visual_revision.wrapping_add(1);
    }
}

/// Validates field geometry and returns its unsigned world-space area.
pub(crate) fn validate_field_polygon_world(polygon_world: &[Vector2]) -> Result<f32, String> {
    validate_player_polygon(polygon_world, "field", MIN_FIELD_POLYGON_AREA_M2)?;
    Ok(field_polygon_area_m2(polygon_world))
}

/// Returns the unsigned world-space area of a field polygon in square metres.
pub(crate) fn field_polygon_area_m2(points: &[Vector2]) -> f32 {
    let mut area = 0.0f32;
    let mut prev = points[points.len() - 1];
    for &curr in points {
        area += prev.x * curr.y - curr.x * prev.y;
        prev = curr;
    }
    (area * 0.5).abs()
}

fn field_area_yield_factor(area_m2: f32) -> f32 {
    if area_m2.is_finite() {
        area_m2.max(0.0) / FIELD_YIELD_BASE_AREA_M2
    } else {
        0.0
    }
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

        assert!((field_polygon_area_m2(&polygon) - 200.0).abs() <= f32::EPSILON);
    }

    #[test]
    fn field_yield_factor_uses_one_hectare_baseline() {
        assert!((field_area_yield_factor(5_000.0) - 0.5).abs() <= f32::EPSILON);
        assert!((field_area_yield_factor(10_000.0) - 1.0).abs() <= f32::EPSILON);
        assert!((field_area_yield_factor(20_000.0) - 2.0).abs() <= f32::EPSILON);
    }
}
