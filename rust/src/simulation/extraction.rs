// SPDX-License-Identifier: GPL-2.0-only

//! Resource extraction sites owned by explicit industry buildings.
//!
//! Authored deposit grids describe where resources exist. Extraction sites are
//! live simulation state: a placed building owns one player-drawn polygon, a
//! reserve snapshot derived from the deposit grid, and a depletion counter.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntimeKind, RuntimeEconomyCatalog, load_runtime_economy_catalog,
};
use crate::simulation::economy::households::{
    OPERATIONAL_HOURS_PER_DAY, building_operation_factors, consume_hourly_production_inputs,
    scaled_output_buffer_capacity_units_for_building,
};
use crate::simulation::resources::{COAL_RESOURCE_ID, ResourceDepositSystem};
use crate::simulation::work_area::{
    explicit_work_area_scale, remove_work_area_owner, top_up_explicit_work_area_startup_budget,
};
use crate::utils::point_in_polygon;
use godot::prelude::Vector2;

/// Maximum accepted gap from the mine footprint to its extraction polygon.
pub(crate) const EXTRACTOR_POLYGON_LINK_DISTANCE_M: f32 = 10.0;
/// Coal units contributed by one square metre of full-richness authored deposit.
pub(crate) const COAL_UNITS_PER_FULL_RICHNESS_M2: f32 = 6.0;

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
    /// Cached unsigned extraction polygon area in square metres.
    pub(crate) area_m2: f32,
    /// Current-area reserve snapshot, floored at this mine's cumulative extracted amount.
    pub(crate) total_reserve_units: f32,
    /// Cumulative extraction by this mine, retained when its polygon changes.
    pub(crate) extracted_units: f32,
}

impl ExtractorSite {
    /// Remaining units that can still be extracted from this site.
    pub(crate) fn remaining_reserve_units(&self) -> f32 {
        (self.total_reserve_units - self.extracted_units).max(0.0)
    }

    fn productive_area_scale(&self) -> f32 {
        if self.remaining_reserve_units() > 0.0 {
            explicit_work_area_scale(self.area_m2)
        } else {
            0.0
        }
    }
}

/// Result returned after creating or replacing an extractor polygon.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ExtractorSiteSummary {
    /// Accepted extraction area in square metres.
    pub(crate) area_m2: f32,
    /// Updated total reserve, including units already extracted before an area change.
    pub(crate) total_reserve_units: f32,
    /// Reserve units remaining after any previous depletion.
    pub(crate) remaining_reserve_units: f32,
}

/// Runtime extraction state for all explicit industry extractor buildings.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResourceExtractionSystem {
    // Unique, ascending owner indices support lookup and shared swap-remove repair.
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
        allocator: &mut BuildingAllocator,
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
        let area_m2 = validate_extractor_polygon_world(&polygon_world)?;
        let site = allocator
            .building_sites
            .get(building_idx)
            .ok_or_else(|| "extractor building site does not exist".to_owned())?;
        validate_polygon_near_building(
            &site.lot_footprint_world,
            &polygon_world,
            "extractor",
            EXTRACTOR_POLYGON_LINK_DISTANCE_M,
        )?;

        let sampled_reserve_units =
            reserve_units_for_resource(&resource_id, deposits, &polygon_world)?;
        validate_extractor_reserve(sampled_reserve_units, 0.0)?;
        let position = self
            .sites
            .binary_search_by_key(&building_idx, |site| site.building_idx);
        let had_site = position.is_ok();
        let extracted_units = position
            .ok()
            .map_or(0.0, |idx| self.sites[idx].extracted_units);
        // Shrinking can exhaust an area, but must not erase already produced coal or
        // let a later expansion grant that same reserve again.
        let total_reserve_units = sampled_reserve_units.max(extracted_units);
        validate_extractor_reserve(total_reserve_units, extracted_units)?;
        let site = ExtractorSite {
            building_idx,
            resource_id,
            polygon_world,
            area_m2,
            total_reserve_units,
            extracted_units,
        };
        let area_scale = site.productive_area_scale();
        match position {
            Ok(idx) => self.sites[idx] = site,
            Err(idx) => self.sites.insert(idx, site),
        }
        self.bump_visual_revision();
        if let Some(building) = allocator.buildings.get_mut(building_idx) {
            building.set_work_area_scale(area_scale);
            if !had_site && let Ok(catalog) = load_runtime_economy_catalog() {
                top_up_explicit_work_area_startup_budget(building, catalog.as_ref(), area_scale);
            }
        }

        Ok(ExtractorSiteSummary {
            area_m2,
            total_reserve_units,
            remaining_reserve_units: total_reserve_units - extracted_units,
        })
    }

    /// Rebuilds cached productive area from committed sites; depleted deposits have no capacity.
    pub(crate) fn apply_work_area_scales(&self, allocator: &mut BuildingAllocator) {
        for site in &self.sites {
            if let Some(building) = allocator.buildings.get_mut(site.building_idx) {
                building.set_work_area_scale(site.productive_area_scale());
            }
        }
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
            let output_headroom = (capacity - current).max(0.0);
            let produced = hourly_units.min(remaining_reserve).min(output_headroom);
            if produced <= 0.0 {
                continue;
            }
            consume_hourly_production_inputs(
                building,
                profile,
                area_factor * factors.throughput_factor * (produced / hourly_units),
            );
            building.add_inventory_units(output_port.resource_runtime_id, produced);
            site.extracted_units += produced;
            building.set_work_area_scale(site.productive_area_scale());
        }
    }

    fn bump_visual_revision(&mut self) {
        self.visual_revision = self.visual_revision.wrapping_add(1);
    }
}

/// Validates authoritative reserve/depletion values before area commit, persistence or restore.
pub(crate) fn validate_extractor_reserve(total: f32, extracted: f32) -> Result<(), String> {
    if total.is_finite()
        && extracted.is_finite()
        && total >= 0.0
        && (0.0..=total).contains(&extracted)
    {
        Ok(())
    } else {
        Err("extractor reserves must be finite and satisfy 0 <= extracted <= total".to_owned())
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

/// Validates extractor geometry and returns its unsigned world-space area.
pub(crate) fn validate_extractor_polygon_world(polygon_world: &[Vector2]) -> Result<f32, String> {
    validate_player_polygon(polygon_world, "extractor", MIN_EXTRACTOR_POLYGON_AREA_M2)
}

/// Validates a player-authored polygon and returns its finite unsigned area in square metres.
pub(crate) fn validate_player_polygon(
    polygon_world: &[Vector2],
    label: &str,
    min_area_m2: f32,
) -> Result<f32, String> {
    if polygon_world.len() < 3 {
        return Err(format!("{label} polygon needs at least three points"));
    }
    for point in polygon_world {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(format!("{label} polygon contains a non-finite point"));
        }
    }
    let area_m2 = polygon_area_abs(polygon_world);
    if !area_m2.is_finite() {
        return Err(format!("{label} polygon area is not finite"));
    }
    if area_m2 < min_area_m2 {
        return Err(format!("{label} polygon is too small"));
    }
    if polygon_has_crossing_edges(polygon_world) {
        return Err(format!("{label} polygon edges cannot cross"));
    }
    Ok(area_m2)
}

/// Validates that a player-authored polygon is linked to a building footprint.
pub(crate) fn validate_polygon_near_building(
    footprint: &[Vector2],
    polygon_world: &[Vector2],
    label: &str,
    max_distance_m: f32,
) -> Result<(), String> {
    let distance = polygon_distance_m(footprint, polygon_world);
    if distance > max_distance_m {
        return Err(format!(
            "{label} polygon must be within {:.0} m of the building",
            max_distance_m
        ));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::resources::RESOURCE_RICHNESS_MAX;

    fn indexed_mine(building_idx: usize) -> ExtractorSite {
        ExtractorSite {
            building_idx,
            resource_id: COAL_RESOURCE_ID.to_owned(),
            polygon_world: vec![Vector2::ZERO, Vector2::RIGHT, Vector2::ONE],
            area_m2: 0.5,
            total_reserve_units: 100_000.0,
            extracted_units: building_idx as f32,
        }
    }

    #[test]
    fn work_area_removal_preserves_owner_state_and_restore_order() {
        use crate::simulation::agriculture::{AgricultureSystem, FieldSite};

        for mask in 0..64 {
            let original: Vec<_> = (0..6)
                .rev()
                .filter(|idx| mask & (1 << idx) != 0)
                .map(indexed_mine)
                .collect();
            for removed in 0..6 {
                let mut mines = ResourceExtractionSystem::from_sites(original.clone());
                let mut farms = AgricultureSystem::from_sites(
                    original
                        .iter()
                        .map(|site| FieldSite {
                            building_idx: site.building_idx,
                            resource_id: "grain".to_owned(),
                            polygon_world: site.polygon_world.clone(),
                            area_m2: site.extracted_units,
                        })
                        .collect(),
                );
                let revision = mines.visual_revision();
                let mut expected: Vec<_> = original
                    .iter()
                    .filter(|site| site.building_idx != removed)
                    .map(|site| {
                        let owner = if site.building_idx == 5 {
                            removed
                        } else {
                            site.building_idx
                        };
                        (owner, site.extracted_units)
                    })
                    .collect();
                expected.sort_unstable_by_key(|&(owner, _)| owner);
                let changed = original
                    .iter()
                    .any(|site| site.building_idx == removed || site.building_idx == 5);
                mines.remove_building_after_swap_remove(removed, 5);
                farms.remove_building_after_swap_remove(removed, 5);
                assert_eq!(mines.visual_revision(), revision + u64::from(changed));
                assert_eq!(farms.visual_revision(), mines.visual_revision());
                assert_eq!(
                    mines
                        .sites()
                        .iter()
                        .map(|site| (site.building_idx, site.extracted_units))
                        .collect::<Vec<_>>(),
                    expected
                );
                assert_eq!(
                    farms
                        .sites()
                        .iter()
                        .map(|site| (site.building_idx, site.area_m2))
                        .collect::<Vec<_>>(),
                    expected
                );
                for owner in 0..6 {
                    let value = expected
                        .iter()
                        .find(|&&(idx, _)| idx == owner)
                        .map(|&(_, value)| value);
                    assert_eq!(
                        mines
                            .site_for_building(owner)
                            .map(|site| site.extracted_units),
                        value
                    );
                    assert_eq!(
                        farms.site_for_building(owner).map(|site| site.area_m2),
                        value
                    );
                }
                let restored = original
                    .iter()
                    .filter(|site| site.building_idx == removed || site.building_idx == 5)
                    .cloned()
                    .collect();
                mines.restore_sites_after_building_removal_undo(removed, 5, restored);
                let reloaded = ResourceExtractionSystem::from_sites(original.clone());
                assert_eq!(
                    mines
                        .sites()
                        .iter()
                        .map(|site| (site.building_idx, site.extracted_units))
                        .collect::<Vec<_>>(),
                    reloaded
                        .sites()
                        .iter()
                        .map(|site| (site.building_idx, site.extracted_units))
                        .collect::<Vec<_>>()
                );
            }
        }
    }

    #[test]
    #[ignore = "manual release timing: cargo test --release benchmark_extraction_site_lookup -- --ignored --nocapture"]
    fn benchmark_extraction_site_lookup() {
        use std::hint::black_box;
        use std::time::Instant;

        for count in [16, 1_024, 65_536] {
            let mut system = ResourceExtractionSystem::from_sites(
                (0..count).map(|idx| indexed_mine(idx * 2)).collect(),
            );
            let queries: Vec<_> = (0..128).map(|idx| idx * 12_289 % (2 * count)).collect();
            let expected: usize = queries
                .iter()
                .filter(|&&idx| idx % 2 == 0)
                .map(|idx| idx + 1)
                .sum();
            for mode in ["lookup", "unrelated_removal"] {
                let mut samples = Vec::with_capacity(21);
                for sample in 0..24 {
                    let start = Instant::now();
                    let mut checksum = 0;
                    for &query in &queries {
                        if mode == "lookup" {
                            checksum += system
                                .site_for_building(black_box(query))
                                .map_or(0, |site| site.building_idx + 1);
                        } else {
                            system.remove_building_after_swap_remove(
                                black_box(count * 2),
                                black_box(count * 2 + 1),
                            );
                        }
                    }
                    let elapsed = start.elapsed().as_secs_f64() * 1_000.0;
                    assert_eq!(checksum, if mode == "lookup" { expected } else { 0 });
                    assert_eq!(system.visual_revision(), 1);
                    black_box(&system);
                    if sample >= 3 {
                        samples.push(elapsed);
                    }
                }
                samples.sort_by(f64::total_cmp);
                eprintln!(
                    "extraction_sites sites={count} mode={mode} queries=128 median_ms={:.6} checksum={expected}",
                    samples[10]
                );
            }
        }
    }

    #[test]
    fn replacing_mine_area_preserves_cumulative_extraction() {
        use crate::simulation::buildings::allocator::indexed_test_building;
        use crate::simulation::economy::definitions::load_runtime_economy_catalog;
        use crate::simulation::zoning::ZoneType;

        let config = WorldConfig::new(40.0, 40.0, 10.0, 10.0);
        let mut deposits = ResourceDepositSystem::from_world_config(&config);
        deposits.set_coal_richness_at(2, 2, RESOURCE_RICHNESS_MAX);
        deposits.set_coal_richness_at(3, 2, RESOURCE_RICHNESS_MAX);
        let manifest = serde_json::from_value(serde_json::json!({
            "asset_id": "mine", "display_name": "Mine fixture",
            "building": {
                "placement_mode": "explicit", "lot_width_cells": 3, "lot_depth_cells": 3,
                "economy_profile": "coal_mine_basic",
                "extractor": {"resource": "coal", "area_mode": "player_polygon"}
            }
        }))
        .unwrap();
        let mut allocator = BuildingAllocator::new();
        allocator.registry.register("test", manifest, String::new());
        let mut mine = indexed_test_building("test:mine".to_owned(), ZoneType::None, 0);
        let catalog = load_runtime_economy_catalog().unwrap();
        mine.economy_profile_runtime_id = catalog
            .profile_for_id("coal_mine_basic")
            .unwrap()
            .runtime_id;
        allocator.buildings.push(mine);
        allocator.push_building_site_client(0, 10.0);
        let polygon = |right| {
            vec![
                Vector2::new(-5.0, -5.0),
                Vector2::new(right, -5.0),
                Vector2::new(right, 5.0),
                Vector2::new(-5.0, 5.0),
            ]
        };
        let mut extraction = ResourceExtractionSystem::new();
        let first = extraction
            .commit_site(0, polygon(15.0), &deposits, &mut allocator)
            .unwrap();
        assert_eq!(first.total_reserve_units, 1_200.0);
        extraction.sites[0].extracted_units = 900.0;
        allocator.buildings[0].operating_budget = 1.0;
        for (right, expected_remaining) in [(15.0, 300.0), (5.0, 0.0), (15.0, 300.0)] {
            let summary = extraction
                .commit_site(0, polygon(right), &deposits, &mut allocator)
                .unwrap();
            let site = extraction.site_for_building(0).unwrap();
            assert_eq!(
                site.extracted_units, 900.0,
                "editing cannot restore extracted coal"
            );
            assert_eq!(site.remaining_reserve_units(), expected_remaining);
            assert_eq!(summary.remaining_reserve_units, expected_remaining);
            assert_eq!(summary.total_reserve_units, expected_remaining + 900.0);
            assert_eq!(
                allocator.buildings[0].work_area_scale > 0.0,
                expected_remaining > 0.0
            );
            assert_eq!(
                allocator.buildings[0].operating_budget, 1.0,
                "edits cannot regrant startup funds"
            );
        }
    }

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
    fn production_polygons_reject_non_finite_area() {
        let polygon = [
            Vector2::new(0.0, 0.0),
            Vector2::new(f32::MAX, 0.0),
            Vector2::new(f32::MAX, f32::MAX),
            Vector2::new(0.0, f32::MAX),
        ];
        assert!(validate_extractor_polygon_world(&polygon).is_err());
        assert!(crate::simulation::agriculture::validate_field_polygon_world(&polygon).is_err());
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

        let err = validate_extractor_polygon_world(&polygon).expect_err("self-crossing polygon");

        assert!(err.contains("edges cannot cross"));
    }
}
