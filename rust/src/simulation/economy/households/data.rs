//! Household record storage and building-reference maintenance.

/// Explicit household runtime record anchored to a residential building.
#[derive(Clone, Debug)]
pub struct Household {
    /// Residential building currently anchoring the household.
    pub home_building_id: usize,
    /// Shared household budget used for essentials in the first-pass loop.
    pub budget: f32,
    /// Current household stock buffer in `household_supplies`.
    pub stock: f32,
    /// Cached linked population count. Rebuilt from resident agents every economy pass.
    pub member_count: u16,
    /// Baseline daily consumption in `household_supplies / day / resident`.
    pub consumption_rate: f32,
    /// Cached derived stock horizon in days at the current consumption rate.
    pub stock_days: f32,
    /// Current replenishment state for diagnostics and cooldown handling.
    pub replenishment_state: u8,
    /// Remaining operational-hour cooldown steps before another replenishment retry.
    pub cooldown_hours: u16,
    /// Reserved source building for the current replenishment request, if any.
    pub reserved_store_building_id: usize,
    /// Reserved amount waiting for household pickup-side fulfillment.
    pub reserved_amount: f32,
    /// Reserved budget waiting to be transferred to the supplying store.
    pub reserved_total_cost: f32,
    /// Remaining operational-hour steps before the reserved pickup completes.
    pub pickup_eta_hours: u16,
    /// Consecutive daily stay-rule failures for the current home.
    pub stay_failure_days: u32,
    /// Consecutive settled days with no valid home after the daily rehousing attempt.
    pub unhoused_days_elapsed: u32,
    /// Stable authored cadence offset used for periodic replenishment checks.
    pub replenishment_offset_hours: u16,
    /// Days elapsed with at least one unemployed member. Resets to 0 when all members are
    /// employed. Incremented each daily tick while the household is benefit-eligible. Once
    /// this reaches `unemployment_max_days`, the household becomes emigration-eligible and
    /// benefit payments stop.
    pub unemployment_days_elapsed: u32,
}

/// Collection of explicit household records for the live simulation.
#[derive(Clone, Debug, Default)]
pub struct HouseholdSystem {
    /// All known households. Agents reference these by index.
    pub households: Vec<Household>,
}

impl HouseholdSystem {
    /// Creates an empty household system.
    pub fn new() -> Self {
        Self {
            households: Vec::new(),
        }
    }

    /// Clears all households.
    pub fn clear(&mut self) {
        self.households.clear();
    }

    /// Remaps building references after a building swap-remove.
    pub fn remap_building_indices(&mut self, mapping: &std::collections::HashMap<usize, usize>) {
        for household in &mut self.households {
            if let Some(&new_id) = mapping.get(&household.home_building_id) {
                household.home_building_id = new_id;
            }
            if let Some(&new_id) = mapping.get(&household.reserved_store_building_id) {
                household.reserved_store_building_id = new_id;
            }
        }
    }

    /// Invalidates references to a building that is being removed.
    pub fn invalidate_building(&mut self, removed_building: usize) {
        for household in &mut self.households {
            if household.home_building_id == removed_building {
                household.home_building_id = usize::MAX;
            }
            if household.reserved_store_building_id == removed_building {
                // Return reserved budget to the household.
                household.budget += household.reserved_total_cost;
                household.reserved_store_building_id = usize::MAX;
                household.reserved_amount = 0.0;
                household.reserved_total_cost = 0.0;
                household.replenishment_state = 0; // REPLENISHMENT_STABLE
            }
        }
    }
}
