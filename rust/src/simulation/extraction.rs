//! Resource extraction sites owned by explicit industry buildings.
//!
//! Authored deposit grids describe where resources exist. Extraction sites are
//! live simulation state: a placed building owns one player-drawn polygon, a
//! reserve snapshot derived from the deposit grid, and a depletion counter.

use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{EconomyProfileRuntimeKind, RuntimeEconomyCatalog};
use crate::simulation::economy::households::building_operation_factors;
use crate::simulation::resources::{COAL_RESOURCE_ID, ResourceDepositSystem};
use godot::prelude::Vector2;

/// Maximum accepted gap from the mine footprint to its extraction polygon.
pub(crate) const EXTRACTOR_POLYGON_LINK_DISTANCE_M: f32 = 10.0;
/// Coal units contributed by one square metre of full-richness authored deposit.
pub(crate) const COAL_UNITS_PER_FULL_RICHNESS_M2: f32 = 6.0;

const OPERATIONAL_HOURS_PER_DAY: f32 = 24.0;
const MIN_EXTRACTOR_POLYGON_AREA_M2: f32 = 1.0;

/// One placed extraction area and its depletion state.
#[derive(Clone, Debug)]
pub(crate) struct ExtractorSite {
    /// Building index that owns this extraction polygon.
    pub(crate) building_idx: usize,
    /// Authored economy resource id, such as `"coal"`.
    pub(crate) resource_id: String,
    /// Player-authored world-space polygon in metres, using `(x, z)`.
    pub(crate) polygon_world: Vec<Vector2>,
    /// Reserve snapshot captured when the polygon was committed.
    pub(crate) total_reserve_units: f32,
    /// Units already extracted from this reserve snapshot.
    pub(crate) extracted_units: f32,
}

impl ExtractorSite {
    /// Remaining units that can still be extracted from this site.
    pub(crate) fn remaining_reserve_units(&self) -> f32 {
        (self.total_reserve_units - self.extracted_units).max(0.0)
    }
}

/// Result returned after creating or replacing an extractor polygon.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ExtractorSiteSummary {
    /// Reserve units sampled from authored deposits when the polygon was committed.
    pub(crate) total_reserve_units: f32,
    /// Reserve units remaining after any previous depletion.
    pub(crate) remaining_reserve_units: f32,
}

/// Runtime extraction state for all explicit industry extractor buildings.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResourceExtractionSystem {
    sites: Vec<ExtractorSite>,
    visual_revision: u64,
}

impl ResourceExtractionSystem {
    /// Creates an empty extraction system.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Restores extraction sites from persistence after validation.
    pub(crate) fn from_sites(mut sites: Vec<ExtractorSite>) -> Self {
        sites.sort_unstable_by_key(|site| site.building_idx);
        let visual_revision = u64::from(!sites.is_empty());
        Self {
            sites,
            visual_revision,
        }
    }

    /// Returns all committed extraction sites.
    pub(crate) fn sites(&self) -> &[ExtractorSite] {
        &self.sites
    }

    /// Monotonic visual-state revision for terrain pit mask refreshes.
    pub(crate) fn visual_revision(&self) -> u64 {
        self.visual_revision
    }

    /// Clears all extraction sites.
    pub(crate) fn clear(&mut self) {
        if self.sites.is_empty() {
            return;
        }
        self.sites.clear();
        self.bump_visual_revision();
    }

    /// Returns the extraction site attached to one building, if present.
    pub(crate) fn site_for_building(&self, building_idx: usize) -> Option<&ExtractorSite> {
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

    /// Restores extractor sites affected by undoing one swap-remove building deletion.
    pub(crate) fn restore_sites_after_building_removal_undo(
        &mut self,
        restored_building_idx: usize,
        last_building_idx_before_remove: usize,
        restored_sites: Vec<ExtractorSite>,
    ) {
        self.sites.retain(|site| {
            site.building_idx != restored_building_idx
                && site.building_idx != last_building_idx_before_remove
        });
        self.sites.extend(restored_sites);
        self.sites.sort_unstable_by_key(|site| site.building_idx);
        self.bump_visual_revision();
    }

    /// Commits or replaces the extraction polygon for one placed extractor building.
    pub(crate) fn commit_site(
        &mut self,
        building_idx: usize,
        polygon_world: Vec<Vector2>,
        deposits: &ResourceDepositSystem,
        allocator: &BuildingAllocator,
        zone_cell_m: f32,
    ) -> Result<ExtractorSiteSummary, String> {
        let building = allocator
            .buildings
            .get(building_idx)
            .ok_or_else(|| "extractor building does not exist".to_owned())?;
        let resource_id = allocator
            .registry
            .extractor_resource(&building.asset_id)
            .ok_or_else(|| "selected building is not a resource extractor".to_owned())?
            .to_owned();
        validate_extractor_polygon(&polygon_world)?;
        validate_polygon_near_building(
            building,
            &polygon_world,
            zone_cell_m,
            "extractor",
            EXTRACTOR_POLYGON_LINK_DISTANCE_M,
        )?;

        let total_reserve_units =
            reserve_units_for_resource(&resource_id, deposits, &polygon_world)?;
        let site = ExtractorSite {
            building_idx,
            resource_id,
            polygon_world,
            total_reserve_units,
            extracted_units: 0.0,
        };
        self.sites.retain(|site| site.building_idx != building_idx);
        self.sites.push(site);
        self.sites.sort_unstable_by_key(|site| site.building_idx);
        self.bump_visual_revision();

        Ok(ExtractorSiteSummary {
            total_reserve_units,
            remaining_reserve_units: total_reserve_units,
        })
    }

    /// Produces resource output from active extractor sites for one operational hour.
    pub(crate) fn produce_hourly(
        &mut self,
        allocator: &mut BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
    ) {
        for site in &mut self.sites {
            let remaining_reserve = site.remaining_reserve_units();
            if remaining_reserve <= 0.0 {
                continue;
            }
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
            if profile.kind != EconomyProfileRuntimeKind::Extractor {
                continue;
            }
            let Some(output_port) = profile.output_port(resource_runtime_id) else {
                continue;
            };
            let factors = building_operation_factors(catalog, building, profile);
            if factors.throughput_factor <= 0.0 {
                continue;
            }
            let hourly_units =
                output_port.units_per_day / OPERATIONAL_HOURS_PER_DAY * factors.throughput_factor;
            if hourly_units <= 0.0 {
                continue;
            }
            let current = building.inventory_units(output_port.resource_runtime_id);
            let capacity = profile.output_buffer_capacity_units_for(output_port);
            let output_headroom = (capacity - current).max(0.0);
            let produced = hourly_units.min(remaining_reserve).min(output_headroom);
            if produced <= 0.0 {
                continue;
            }
            building.add_inventory_units(output_port.resource_runtime_id, produced);
            site.extracted_units += produced;
        }
    }

    fn bump_visual_revision(&mut self) {
        self.visual_revision = self.visual_revision.wrapping_add(1);
    }
}

fn reserve_units_for_resource(
    resource_id: &str,
    deposits: &ResourceDepositSystem,
    polygon_world: &[Vector2],
) -> Result<f32, String> {
    match resource_id {
        COAL_RESOURCE_ID => {
            Ok(deposits
                .coal_reserve_units_for_polygon(polygon_world, COAL_UNITS_PER_FULL_RICHNESS_M2))
        }
        other => Err(format!(
            "resource extractor deposits for '{other}' are not implemented yet"
        )),
    }
}

fn validate_extractor_polygon(polygon_world: &[Vector2]) -> Result<(), String> {
    validate_player_polygon(polygon_world, "extractor", MIN_EXTRACTOR_POLYGON_AREA_M2)
}

/// Validates a player-authored polygon for one linked building area.
pub(crate) fn validate_player_polygon(
    polygon_world: &[Vector2],
    label: &str,
    min_area_m2: f32,
) -> Result<(), String> {
    if polygon_world.len() < 3 {
        return Err(format!("{label} polygon needs at least three points"));
    }
    for point in polygon_world {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(format!("{label} polygon contains a non-finite point"));
        }
    }
    if polygon_area_abs(polygon_world) < min_area_m2 {
        return Err(format!("{label} polygon is too small"));
    }
    if polygon_has_crossing_edges(polygon_world) {
        return Err(format!("{label} polygon edges cannot cross"));
    }
    Ok(())
}

/// Validates that a player-authored polygon is linked to a building footprint.
pub(crate) fn validate_polygon_near_building(
    building: &Building,
    polygon_world: &[Vector2],
    zone_cell_m: f32,
    label: &str,
    max_distance_m: f32,
) -> Result<(), String> {
    let footprint = building_footprint_polygon(building, zone_cell_m);
    let distance = polygon_distance_m(&footprint, polygon_world);
    if distance > max_distance_m {
        return Err(format!(
            "{label} polygon must be within {:.0} m of the building",
            max_distance_m
        ));
    }
    Ok(())
}

/// Returns the world-space footprint polygon used to link extraction sites to a building.
pub(crate) fn building_footprint_polygon(building: &Building, zone_cell_m: f32) -> [Vector2; 4] {
    let safe_cell_m = zone_cell_m.max(f32::EPSILON);
    let half_width = f32::from(building.width_cells) * safe_cell_m * 0.5;
    let half_depth = f32::from(building.depth_cells) * safe_cell_m * 0.5;
    let center = Vector2::new(building.center_x, building.center_y);
    let facing = if building.facing_dir.length_squared() > 1e-8 {
        building.facing_dir.normalized()
    } else {
        Vector2::new(0.0, -1.0)
    };
    let depth_axis = -facing;
    let width_axis = Vector2::new(-depth_axis.y, depth_axis.x);
    [
        center - width_axis * half_width - depth_axis * half_depth,
        center + width_axis * half_width - depth_axis * half_depth,
        center + width_axis * half_width + depth_axis * half_depth,
        center - width_axis * half_width + depth_axis * half_depth,
    ]
}

fn polygon_area_abs(points: &[Vector2]) -> f32 {
    let mut area = 0.0f32;
    let mut prev = points[points.len() - 1];
    for &curr in points {
        area += prev.x * curr.y - curr.x * prev.y;
        prev = curr;
    }
    (area * 0.5).abs()
}

fn polygon_has_crossing_edges(points: &[Vector2]) -> bool {
    let count = points.len();
    for first_edge in 0..count {
        let a0 = points[first_edge];
        let a1 = points[(first_edge + 1) % count];
        for second_edge in (first_edge + 1)..count {
            if polygon_edges_are_adjacent(first_edge, second_edge, count) {
                continue;
            }
            let b0 = points[second_edge];
            let b1 = points[(second_edge + 1) % count];
            if segments_intersect(a0, a1, b0, b1) {
                return true;
            }
        }
    }
    false
}

fn polygon_edges_are_adjacent(left: usize, right: usize, edge_count: usize) -> bool {
    (left + 1) % edge_count == right || (right + 1) % edge_count == left
}

fn polygon_distance_m(left: &[Vector2], right: &[Vector2]) -> f32 {
    if left.iter().any(|point| point_in_polygon(*point, right))
        || right.iter().any(|point| point_in_polygon(*point, left))
    {
        return 0.0;
    }
    let mut best = f32::INFINITY;
    for_each_segment(left, |a0, a1| {
        for_each_segment(right, |b0, b1| {
            best = best.min(segment_segment_distance(a0, a1, b0, b1));
        });
    });
    best
}

fn for_each_segment(points: &[Vector2], mut callback: impl FnMut(Vector2, Vector2)) {
    let mut prev = points[points.len() - 1];
    for &curr in points {
        callback(prev, curr);
        prev = curr;
    }
}

fn segment_segment_distance(a0: Vector2, a1: Vector2, b0: Vector2, b1: Vector2) -> f32 {
    if segments_intersect(a0, a1, b0, b1) {
        return 0.0;
    }
    point_segment_distance(a0, b0, b1)
        .min(point_segment_distance(a1, b0, b1))
        .min(point_segment_distance(b0, a0, a1))
        .min(point_segment_distance(b1, a0, a1))
}

fn point_segment_distance(point: Vector2, start: Vector2, end: Vector2) -> f32 {
    let segment = end - start;
    let len_sq = segment.length_squared();
    if len_sq <= f32::EPSILON {
        return point.distance_to(start);
    }
    let t = ((point - start).dot(segment) / len_sq).clamp(0.0, 1.0);
    point.distance_to(start + segment * t)
}

fn segments_intersect(a0: Vector2, a1: Vector2, b0: Vector2, b1: Vector2) -> bool {
    let d1 = orientation(a0, a1, b0);
    let d2 = orientation(a0, a1, b1);
    let d3 = orientation(b0, b1, a0);
    let d4 = orientation(b0, b1, a1);
    if d1.abs() <= f32::EPSILON && point_on_segment(b0, a0, a1) {
        return true;
    }
    if d2.abs() <= f32::EPSILON && point_on_segment(b1, a0, a1) {
        return true;
    }
    if d3.abs() <= f32::EPSILON && point_on_segment(a0, b0, b1) {
        return true;
    }
    if d4.abs() <= f32::EPSILON && point_on_segment(a1, b0, b1) {
        return true;
    }
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

fn orientation(a: Vector2, b: Vector2, c: Vector2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn point_on_segment(point: Vector2, start: Vector2, end: Vector2) -> bool {
    point.x >= start.x.min(end.x) - f32::EPSILON
        && point.x <= start.x.max(end.x) + f32::EPSILON
        && point.y >= start.y.min(end.y) - f32::EPSILON
        && point.y <= start.y.max(end.y) + f32::EPSILON
}

fn point_in_polygon(point: Vector2, polygon: &[Vector2]) -> bool {
    let mut inside = false;
    let mut prev = polygon[polygon.len() - 1];
    for &curr in polygon {
        let crosses = (curr.y > point.y) != (prev.y > point.y);
        if crosses {
            let denom = prev.y - curr.y;
            if denom.abs() > f32::EPSILON {
                let intersection_x = (prev.x - curr.x) * (point.y - curr.y) / denom + curr.x;
                if point.x < intersection_x {
                    inside = !inside;
                }
            }
        }
        prev = curr;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::resources::RESOURCE_RICHNESS_MAX;

    #[test]
    fn coal_reserve_uses_polygon_area_and_richness() {
        let config = WorldConfig::new(40.0, 40.0, 10.0, 10.0)
            .with_terrain_resolution(10.0)
            .with_chunking(20.0, 0.0);
        let mut deposits = ResourceDepositSystem::from_world_config(&config);
        deposits.set_coal_richness_at(2, 2, RESOURCE_RICHNESS_MAX);
        deposits.set_coal_richness_at(3, 2, RESOURCE_RICHNESS_MAX / 2);
        let polygon = vec![
            Vector2::new(-5.0, -5.0),
            Vector2::new(15.0, -5.0),
            Vector2::new(15.0, 5.0),
            Vector2::new(-5.0, 5.0),
        ];

        let reserve = deposits.coal_reserve_units_for_polygon(&polygon, 2.0);

        assert!((reserve - 300.0).abs() <= 0.001);
    }

    #[test]
    fn extractor_polygon_rejects_crossing_edges() {
        let polygon = vec![
            Vector2::new(0.0, 0.0),
            Vector2::new(10.0, 0.0),
            Vector2::new(10.0, 10.0),
            Vector2::new(5.0, -2.0),
            Vector2::new(0.0, 10.0),
        ];

        let err = validate_extractor_polygon(&polygon).expect_err("self-crossing polygon");

        assert!(err.contains("edges cannot cross"));
    }
}
