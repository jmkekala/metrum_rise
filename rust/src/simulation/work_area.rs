//! Shared scaling rules for explicit player-drawn production areas.

use crate::simulation::buildings::allocator::Building;
use crate::simulation::economy::definitions::EconomyProfileRuntimeKind;
use crate::simulation::economy::definitions::RuntimeEconomyCatalog;
use crate::simulation::zoning::ZoneType;

/// Production-area size that receives exactly the authored worker and output rates.
pub(crate) const EXPLICIT_WORK_AREA_BASE_M2: f32 = 10_000.0;
/// Payroll runway granted when an explicit production area is first committed.
pub(crate) const EXPLICIT_WORK_AREA_STARTUP_RUNWAY_DAYS: f32 = 7.0;
/// Minimum startup operating budget for explicit area-backed producers.
pub(crate) const EXPLICIT_WORK_AREA_STARTUP_MIN_BUDGET: f32 = 500.0;

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

/// Converts authored full-area worker slots into active slots for one explicit area.
pub(crate) fn scaled_work_area_worker_capacity(worker_capacity: u32, area_scale: f32) -> u32 {
    if worker_capacity == 0 {
        return 0;
    }
    let area_scale = sanitize_work_area_scale(area_scale);
    if area_scale <= f32::EPSILON {
        return 0;
    }
    let scaled_capacity = (worker_capacity as f32 * area_scale).ceil();
    if scaled_capacity >= u32::MAX as f32 {
        u32::MAX
    } else {
        scaled_capacity as u32
    }
}

/// Returns the initial operating budget needed for the area-scaled active workforce.
pub(crate) fn explicit_work_area_startup_budget(
    worker_capacity: u32,
    average_daily_wage: f32,
    area_scale: f32,
) -> f32 {
    let active_worker_capacity = scaled_work_area_worker_capacity(worker_capacity, area_scale);
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
        profile.worker_capacity,
        profile.average_daily_wage(),
        area_scale,
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
        assert_eq!(scaled_work_area_worker_capacity(10, 0.0), 0);
        assert_eq!(scaled_work_area_worker_capacity(10, 0.01), 1);
        assert_eq!(scaled_work_area_worker_capacity(10, 0.2731), 3);
        assert_eq!(scaled_work_area_worker_capacity(10, 2.0), 20);
    }

    #[test]
    fn startup_budget_uses_scaled_worker_capacity() {
        assert_eq!(explicit_work_area_startup_budget(10, 90.0, 0.0), 500.0);
        assert_eq!(explicit_work_area_startup_budget(10, 90.0, 0.5), 3150.0);
        assert_eq!(explicit_work_area_startup_budget(16, 90.0, 0.25), 2520.0);
    }
}
