// SPDX-License-Identifier: GPL-2.0-only

//! Shared scaling and owner-remapping rules for explicit player-drawn production areas.

use crate::simulation::buildings::allocator::Building;
use crate::simulation::economy::definitions::EconomyProfileRuntimeKind;
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, load_runtime_economy_tuning};
use crate::simulation::zoning::ZoneType;

/// Production-area size that receives exactly the authored output rates.
pub(crate) const EXPLICIT_WORK_AREA_BASE_M2: f32 = 10_000.0;
/// Payroll runway granted when an explicit production area is first committed.
pub(crate) const EXPLICIT_WORK_AREA_STARTUP_RUNWAY_DAYS: f32 = 7.0;
/// Minimum startup operating budget for explicit area-backed producers.
pub(crate) const EXPLICIT_WORK_AREA_STARTUP_MIN_BUDGET: f32 = 500.0;
/// Minimum physical workforce for a committed field with an enabled staffing profile.
pub(crate) const MIN_FIELD_WORKERS: u32 = 2;

/// Removes and remaps production sites after their owning building store swap-removes an entry.
///
/// Sites must have unique, ascending building indices. Unrelated removals cost O(log S);
/// affected sites retain that ordering with O(S) vector shifts and no allocation.
pub(crate) fn remove_work_area_owner<T>(
    sites: &mut Vec<T>,
    removed: usize,
    last_before_remove: usize,
    owner: impl Fn(&T) -> usize,
    set_owner: impl Fn(&mut T, usize),
) -> bool {
    let mut changed = false;
    if let Ok(idx) = sites.binary_search_by_key(&removed, &owner) {
        sites.remove(idx);
        changed = true;
    }
    if removed != last_before_remove
        && sites
            .last()
            .is_some_and(|site| owner(site) == last_before_remove)
        && let Some(mut site) = sites.pop()
    {
        set_owner(&mut site, removed);
        let idx = sites.partition_point(|site| owner(site) < removed);
        sites.insert(idx, site);
        changed = true;
    }
    changed
}

/// Returns the linear scale for one explicit production area.
pub(crate) fn explicit_work_area_scale(area_m2: f32) -> f32 {
    if area_m2.is_finite() {
        area_m2.max(0.0) / EXPLICIT_WORK_AREA_BASE_M2
    } else {
        0.0
    }
}

/// Returns a non-negative finite area scale suitable for cached building state.
pub(crate) fn sanitize_work_area_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.max(0.0)
    } else {
        0.0
    }
}

/// Returns true when an economy profile uses a player-drawn production area.
pub(crate) fn profile_kind_uses_explicit_work_area(kind: EconomyProfileRuntimeKind) -> bool {
    matches!(
        kind,
        EconomyProfileRuntimeKind::FieldProducer | EconomyProfileRuntimeKind::Extractor
    )
}

/// Returns the field-only staffing floor shared by payroll, production and demand.
pub(crate) fn minimum_work_area_workers(kind: EconomyProfileRuntimeKind) -> u32 {
    if kind == EconomyProfileRuntimeKind::FieldProducer {
        MIN_FIELD_WORKERS
    } else {
        0
    }
}

/// Returns the initial cached area scale for a newly created or loaded building.
pub(crate) fn initial_work_area_scale(
    zone_type: ZoneType,
    profile_kind: Option<EconomyProfileRuntimeKind>,
) -> f32 {
    if zone_type == ZoneType::None && profile_kind.is_some_and(profile_kind_uses_explicit_work_area)
    {
        0.0
    } else {
        1.0
    }
}

/// Applies a workforce floor to positive production areas, retaining the authored density above it.
pub(crate) fn work_area_worker_equivalent(
    workers_per_hectare: f32,
    area_scale: f32,
    minimum_workers: u32,
) -> f32 {
    if workers_per_hectare <= 0.0 {
        return 0.0;
    }
    let area_scale = sanitize_work_area_scale(area_scale);
    if area_scale <= f32::EPSILON {
        return 0.0;
    }
    (workers_per_hectare * area_scale).max(minimum_workers as f32)
}

/// Rounds the workforce equivalent up to whole slots; uncommitted areas still have zero jobs.
pub(crate) fn scaled_work_area_worker_capacity(
    workers_per_hectare: f32,
    area_scale: f32,
    minimum_workers: u32,
) -> u32 {
    let scaled_capacity =
        work_area_worker_equivalent(workers_per_hectare, area_scale, minimum_workers).ceil();
    if scaled_capacity >= u32::MAX as f32 {
        u32::MAX
    } else {
        scaled_capacity as u32
    }
}

/// Returns the initial operating budget needed for the area-scaled active workforce.
pub(crate) fn explicit_work_area_startup_budget(
    workers_per_hectare: f32,
    average_daily_wage: f32,
    area_scale: f32,
    minimum_workers: u32,
) -> f32 {
    let active_worker_capacity =
        scaled_work_area_worker_capacity(workers_per_hectare, area_scale, minimum_workers);
    (active_worker_capacity as f32
        * average_daily_wage.max(0.0)
        * EXPLICIT_WORK_AREA_STARTUP_RUNWAY_DAYS)
        .max(EXPLICIT_WORK_AREA_STARTUP_MIN_BUDGET)
}

/// Raises a newly committed explicit-area producer to its area-scaled startup runway.
pub(crate) fn top_up_explicit_work_area_startup_budget(
    building: &mut Building,
    catalog: &RuntimeEconomyCatalog,
    area_scale: f32,
) {
    let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id) else {
        return;
    };
    if !profile_kind_uses_explicit_work_area(profile.kind) {
        return;
    }
    let target = explicit_work_area_startup_budget(
        profile.workers_per_hectare,
        profile.average_daily_wage(),
        area_scale,
        minimum_work_area_workers(profile.kind),
    ) + profile.initial_input_import_cost(
        catalog,
        area_scale,
        load_runtime_economy_tuning()
            .expect("runtime tuning loaded")
            .owa_import_price_multiplier,
    );
    if building.operating_budget >= target {
        return;
    }
    let top_up = target - building.operating_budget;
    building.operating_budget += top_up;
    building.profit_tax_budget_baseline += top_up;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_work_area_scale_uses_one_hectare_baseline() {
        assert!((explicit_work_area_scale(5_000.0) - 0.5).abs() <= f32::EPSILON);
        assert!((explicit_work_area_scale(10_000.0) - 1.0).abs() <= f32::EPSILON);
        assert!((explicit_work_area_scale(20_000.0) - 2.0).abs() <= f32::EPSILON);
    }

    #[test]
    fn scaled_worker_capacity_ceilings_small_positive_areas() {
        assert_eq!(scaled_work_area_worker_capacity(10.0, 0.0, 0), 0);
        assert_eq!(scaled_work_area_worker_capacity(10.0, 0.01, 0), 1);
        assert_eq!(scaled_work_area_worker_capacity(10.0, 0.2731, 0), 3);
        assert_eq!(scaled_work_area_worker_capacity(10.0, 2.0, 0), 20);
    }

    #[test]
    fn field_worker_minimum_keeps_existing_density_above_two() {
        for (hectares, workers) in [
            (0.0, 0),
            (0.01, 2),
            (10.0, 2),
            (20.0, 2),
            (20.01, 3),
            (30.0, 3),
            (100.0, 10),
        ] {
            assert_eq!(
                scaled_work_area_worker_capacity(0.1, hectares, MIN_FIELD_WORKERS),
                workers
            );
        }
        assert_eq!(
            scaled_work_area_worker_capacity(0.0, 10.0, MIN_FIELD_WORKERS),
            0
        );
        assert_eq!(
            scaled_work_area_worker_capacity(0.1, f32::NAN, MIN_FIELD_WORKERS),
            0
        );
    }

    #[test]
    fn startup_budget_uses_scaled_worker_capacity() {
        assert_eq!(explicit_work_area_startup_budget(10.0, 90.0, 0.0, 0), 500.0);
        assert_eq!(
            explicit_work_area_startup_budget(10.0, 90.0, 0.5, 0),
            3150.0
        );
        assert_eq!(
            explicit_work_area_startup_budget(16.0, 90.0, 0.25, 0),
            2520.0
        );
        assert_eq!(
            explicit_work_area_startup_budget(0.1, 90.0, 1.0, MIN_FIELD_WORKERS),
            1260.0
        );
    }

    #[test]
    #[ignore = "manual release timing: cargo test --release benchmark_work_area_capacity -- --ignored --nocapture"]
    fn benchmark_work_area_capacity() {
        use std::hint::black_box;
        use std::time::Instant;

        let areas = [0.0, 0.01, 0.2731, 1.0, 10.0, 10.01, 20.0, 100.0];
        for (label, minimum_workers, cycle_total) in
            [("density", 0, 18), ("farm", MIN_FIELD_WORKERS, 22)]
        {
            for count in [10_000, 100_000, 1_000_000] {
                let mut samples = [0.0; 21];
                for sample in &mut samples {
                    let start = Instant::now();
                    let mut total = 0u64;
                    for i in 0..count {
                        total += scaled_work_area_worker_capacity(
                            black_box(0.1),
                            black_box(areas[i % areas.len()]),
                            black_box(minimum_workers),
                        ) as u64;
                    }
                    *sample = start.elapsed().as_secs_f64();
                    assert_eq!(total, (count / areas.len() * cycle_total) as u64);
                    black_box(total);
                }
                samples.sort_by(f64::total_cmp);
                eprintln!(
                    "work_area_capacity mode={label} queries={count} median_ms={:.3} ns_per_query={:.2}",
                    samples[10] * 1000.0,
                    samples[10] * 1e9 / count as f64
                );
            }
        }
    }
}
