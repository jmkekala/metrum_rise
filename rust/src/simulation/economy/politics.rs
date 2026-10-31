// SPDX-License-Identifier: GPL-2.0-only

//! Political factions, policy approval dynamics, and municipal election cycle (`FactionManager`).
//!
//! Models four distinct political factions that react dynamically to Land Value Tax (LVT),
//! Citizen's Dividend (UBI) payouts, reservation-wage-driven labor shortages, conventional
//! taxes, environmental quality (pollution and noise), and pre-election policy shocks.

/// Number of political factions managed by [`FactionManager`].
pub const FACTION_COUNT: usize = 4;

/// Default municipal election cycle length in operational days.
pub const DEFAULT_ELECTION_CYCLE_DAYS: u32 = 180;

/// Identifier for each political faction in the city.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum FactionId {
    /// Landowners, real-estate holders, and high-level building proprietors.
    PropertyOwners = 0,
    /// Wage earners, young adults, renters, and unemployed residents.
    WorkersAndYouth = 1,
    /// Business operators and industrial/commercial employers.
    Industrialists = 2,
    /// Residents prioritizing low pollution, low noise, and compact land use.
    Environmentalists = 3,
}

impl FactionId {
    /// All factions in canonical index order.
    pub const ALL: [Self; FACTION_COUNT] = [
        Self::PropertyOwners,
        Self::WorkersAndYouth,
        Self::Industrialists,
        Self::Environmentalists,
    ];

    /// Zero-based array index for this faction.
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Stable snake_case identifier for UI dictionaries.
    #[inline]
    pub const fn id_str(self) -> &'static str {
        match self {
            Self::PropertyOwners => "property_owners",
            Self::WorkersAndYouth => "workers_and_youth",
            Self::Industrialists => "industrialists",
            Self::Environmentalists => "environmentalists",
        }
    }

    /// Display name for UI panels.
    #[inline]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::PropertyOwners => "Property Owners",
            Self::WorkersAndYouth => "Workers & Youth",
            Self::Industrialists => "Industrialists",
            Self::Environmentalists => "Environmentalists",
        }
    }
}

/// Dominant policy or city condition driving a faction's current approval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum FactionIssue {
    /// No single acute grievance or praise factor dominates.
    #[default]
    Neutral = 0,
    /// Reaction to the Land Value Tax (LVT) rate.
    LvtBurden = 1,
    /// Reaction to the Citizen's Dividend (UBI) payout.
    UbiDividend = 2,
    /// Reaction to reservation-wage-driven labor shortages in private sectors.
    LaborShortage = 3,
    /// Reaction to income, VAT, profit, or building property taxes and policy shocks.
    TaxReform = 4,
    /// Reaction to environmental pollution and traffic noise.
    PollutionAndNoise = 5,
    /// Reaction to municipal utility/power service reliability.
    ServiceReliability = 6,
}

impl FactionIssue {
    /// Stable snake_case identifier for UI dictionaries.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::LvtBurden => "lvt_burden",
            Self::UbiDividend => "ubi_dividend",
            Self::LaborShortage => "labor_shortage",
            Self::TaxReform => "tax_reform",
            Self::PollutionAndNoise => "pollution_and_noise",
            Self::ServiceReliability => "service_reliability",
        }
    }
}

/// State of a single political faction, kept in a fixed-size array `[FactionState; FACTION_COUNT]`.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FactionState {
    /// Population / electorate share (`0.0..=1.0`, sums to `1.0` across all factions).
    pub electorate_share: f32,
    /// Instantaneous target approval (`0.0..=100.0`) derived from live policies and city metrics.
    pub target_approval: f32,
    /// Smoothed approval rating (`0.0..=100.0`) with exponential moving average inertia.
    pub approval: f32,
    /// Pre-election sensitivity multiplier (`1.0..=2.0`), ramping up as elections near.
    pub campaign_volatility: f32,
    /// Primary issue currently driving this faction's approval.
    pub dominant_issue: FactionIssue,
}

impl Default for FactionState {
    fn default() -> Self {
        Self {
            electorate_share: 0.25,
            target_approval: 60.0,
            approval: 60.0,
            campaign_volatility: 1.0,
            dominant_issue: FactionIssue::Neutral,
        }
    }
}

/// Compact, allocation-free snapshot of citywide economic and environmental metrics consumed by
/// [`FactionManager`].
#[derive(Clone, Copy, Debug, Default)]
pub struct PoliticsUpdateInputs {
    /// Land Value Tax daily rate (`0.0..=1.0`).
    pub lvt_rate: f32,
    /// Normalized conventional building property tax burden (`0.0..=1.0`).
    pub property_tax_burden: f32,
    /// Combined household income tax and VAT rate (`0.0..=1.25`).
    pub income_and_vat_rate: f32,
    /// Business profit tax rate (`0.0..=0.75`).
    pub business_profit_tax_rate: f32,
    /// Active Citizen's Dividend paid per adult per day.
    pub daily_ubi_per_adult: f32,
    /// Daily subsistence cost per adult resident.
    pub base_subsistence_wage: f32,
    /// Unfilled vacancy ratio in low-wage commercial workplaces (`0.0..=1.0`).
    pub low_wage_shortage_ratio: f32,
    /// Unfilled vacancy ratio in industrial workplaces (`0.0..=1.0`).
    pub industrial_shortage_ratio: f32,
    /// Citywide mean pollution level (`0.0..=100.0`).
    pub average_pollution: f32,
    /// Citywide mean traffic/building noise level (`0.0..=100.0`).
    pub average_noise: f32,
    /// Local electricity service coverage ratio (`0.0..=1.0`).
    pub power_coverage: f32,
    /// Demographic weight for [`FactionId::PropertyOwners`].
    pub property_owner_weight: f32,
    /// Demographic weight for [`FactionId::WorkersAndYouth`].
    pub worker_youth_weight: f32,
    /// Demographic weight for [`FactionId::Industrialists`].
    pub industrialist_weight: f32,
    /// Demographic weight for [`FactionId::Environmentalists`].
    pub environmentalist_weight: f32,
}

/// Manages all political factions, approval smoothing, policy reform shocks, and the municipal
/// election cycle.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FactionManager {
    /// Contiguous fixed-size array of the 4 factions indexed by [`FactionId`].
    pub factions: [FactionState; FACTION_COUNT],
    /// Length of one municipal election term in operational days.
    pub election_cycle_days: u32,
    /// Operational days remaining until the next municipal election.
    pub days_until_election: u32,
    /// Electorate-weighted overall popular approval (`0.0..=100.0`).
    pub overall_approval: f32,
    /// Total municipal elections completed since city founding.
    pub elections_completed: u32,
    /// Total municipal elections won (`overall_approval >= 50.0`).
    pub elections_won: u32,
    /// Vote share (`0.0..=100.0`) recorded at the most recent municipal election.
    pub last_election_vote_share: f32,
    /// Whether the administration won a majority at the most recent election.
    pub last_election_won: bool,
    /// Transient policy-reform shock accumulator (spikes on abrupt tax changes near elections).
    pub tax_reform_shock: f32,
}

impl Default for FactionManager {
    fn default() -> Self {
        Self {
            factions: [FactionState::default(); FACTION_COUNT],
            election_cycle_days: DEFAULT_ELECTION_CYCLE_DAYS,
            days_until_election: DEFAULT_ELECTION_CYCLE_DAYS,
            overall_approval: 60.0,
            elections_completed: 0,
            elections_won: 0,
            last_election_vote_share: 60.0,
            last_election_won: true,
            tax_reform_shock: 0.0,
        }
    }
}

impl FactionManager {
    /// Records an immediate policy change shock when tax or transfer rates are modified.
    /// Changes made shortly before an election carry higher campaign volatility. O(1).
    pub fn record_policy_change(&mut self, normalized_delta: f32) {
        if !normalized_delta.is_finite() || normalized_delta <= 0.0 {
            return;
        }
        let volatility = self.current_campaign_volatility();
        self.tax_reform_shock =
            (self.tax_reform_shock + normalized_delta * 20.0 * volatility).clamp(0.0, 30.0);
    }

    /// Computes the current pre-election campaign volatility multiplier (`1.0..=2.0`). O(1).
    #[inline]
    pub fn current_campaign_volatility(&self) -> f32 {
        let window = 30.0_f32;
        let days = (self.days_until_election as f32).min(window);
        1.0 + ((window - days) / window).clamp(0.0, 1.0)
    }

    /// Re-evaluates electorate shares, instantaneous target approvals, and dominant issues from
    /// `inputs` without advancing the daily clock. O(1).
    pub fn evaluate_targets(&mut self, inputs: &PoliticsUpdateInputs) {
        let volatility = self.current_campaign_volatility();
        let raw_weights = [
            inputs.property_owner_weight.max(1.0),
            inputs.worker_youth_weight.max(1.0),
            inputs.industrialist_weight.max(1.0),
            inputs.environmentalist_weight.max(1.0),
        ];
        let weight_sum: f32 = raw_weights.iter().sum::<f32>().max(1.0);

        let lvt = inputs.lvt_rate.clamp(0.0, 1.0);
        let prop_tax = inputs.property_tax_burden.clamp(0.0, 1.0);
        let regressive_tax = inputs.income_and_vat_rate.clamp(0.0, 1.25);
        let profit_tax = inputs.business_profit_tax_rate.clamp(0.0, 0.75);
        let ubi_ratio =
            (inputs.daily_ubi_per_adult.max(0.0) / inputs.base_subsistence_wage.max(1.0)).min(2.0);
        let shortage =
            (0.6 * inputs.low_wage_shortage_ratio + 0.4 * inputs.industrial_shortage_ratio)
                .clamp(0.0, 1.0);
        let env_burden =
            (0.6 * inputs.average_pollution + 0.4 * inputs.average_noise).clamp(0.0, 100.0);
        let power = inputs.power_coverage.clamp(0.0, 1.0);
        let shock = self.tax_reform_shock;

        // 1. Property Owners: strongly oppose LVT and property taxes; favor infrastructure/power.
        let po_lvt_delta = 10.0 * (1.0 - lvt * 4.0) - 65.0 * lvt * volatility;
        let po_tax_delta = -25.0 * prop_tax - 0.6 * shock;
        let po_srv_delta = 8.0 * (power - 0.5);
        let po_target = (58.0 + po_lvt_delta + po_tax_delta + po_srv_delta).clamp(0.0, 100.0);
        let po_issue = if lvt >= 0.15 {
            FactionIssue::LvtBurden
        } else if prop_tax >= 0.25 || shock >= 8.0 {
            FactionIssue::TaxReform
        } else if power < 0.8 {
            FactionIssue::ServiceReliability
        } else {
            FactionIssue::Neutral
        };

        // 2. Workers & Youth: strongly favor UBI and low wage/VAT taxes; mildly favor LVT.
        let wy_ubi_delta = 42.0 * ubi_ratio * volatility;
        let wy_lvt_delta = 14.0 * lvt;
        let wy_tax_delta = -45.0 * regressive_tax - 0.3 * shock;
        let wy_target = (52.0 + wy_ubi_delta + wy_lvt_delta + wy_tax_delta).clamp(0.0, 100.0);
        let wy_issue = if ubi_ratio >= 0.20 {
            FactionIssue::UbiDividend
        } else if regressive_tax >= 0.25 {
            FactionIssue::TaxReform
        } else if lvt >= 0.20 {
            FactionIssue::LvtBurden
        } else {
            FactionIssue::Neutral
        };

        // 3. Industrialists: strongly oppose labor shortages (from high UBI reservation wage) and
        // profit taxes; favor Georgist untaxing of production and reliable power.
        let ind_shortage_delta = -60.0 * shortage * volatility;
        let ind_tax_delta =
            -45.0 * profit_tax - 20.0 * prop_tax + 12.0 * lvt * (1.0 - profit_tax) - 0.4 * shock;
        let ind_srv_delta = 14.0 * (power - 0.5);
        let ind_target =
            (58.0 + ind_shortage_delta + ind_tax_delta + ind_srv_delta).clamp(0.0, 100.0);
        let ind_issue = if shortage >= 0.15 {
            FactionIssue::LaborShortage
        } else if profit_tax >= 0.25 || prop_tax >= 0.25 {
            FactionIssue::TaxReform
        } else if power < 0.85 {
            FactionIssue::ServiceReliability
        } else {
            FactionIssue::Neutral
        };

        // 4. Environmentalists: favor LVT (compact land use) and UBI; strongly oppose pollution/noise.
        let env_lvt_delta = 32.0 * lvt + 8.0 * ubi_ratio.min(1.0);
        let env_pol_delta = -0.85 * env_burden * volatility;
        let env_target = (60.0 + env_lvt_delta + env_pol_delta).clamp(0.0, 100.0);
        let env_issue = if env_burden >= 20.0 {
            FactionIssue::PollutionAndNoise
        } else if lvt >= 0.15 {
            FactionIssue::LvtBurden
        } else {
            FactionIssue::Neutral
        };

        let targets = [
            (po_target, po_issue),
            (wy_target, wy_issue),
            (ind_target, ind_issue),
            (env_target, env_issue),
        ];
        for (i, &(target, issue)) in targets.iter().enumerate() {
            self.factions[i].electorate_share = raw_weights[i] / weight_sum;
            self.factions[i].target_approval = target;
            self.factions[i].campaign_volatility = volatility;
            self.factions[i].dominant_issue = issue;
        }
        self.recompute_overall_approval();
    }

    /// Advances the political simulation by one operational day: evaluates targets, applies
    /// exponential smoothing to faction approvals, decays `tax_reform_shock`, and resolves the
    /// municipal election when `days_until_election` reaches zero. O(1).
    pub fn advance_day(&mut self, inputs: &PoliticsUpdateInputs) {
        self.evaluate_targets(inputs);
        let smoothing = 0.25_f32;
        for faction in &mut self.factions {
            faction.approval = (faction.approval
                + smoothing * (faction.target_approval - faction.approval))
                .clamp(0.0, 100.0);
        }
        self.recompute_overall_approval();
        self.tax_reform_shock = (self.tax_reform_shock * 0.85).max(0.0);

        if self.days_until_election > 1 {
            self.days_until_election -= 1;
        } else {
            self.elections_completed = self.elections_completed.saturating_add(1);
            self.last_election_vote_share = self.overall_approval;
            self.last_election_won = self.overall_approval >= 50.0;
            if self.last_election_won {
                self.elections_won = self.elections_won.saturating_add(1);
            }
            self.days_until_election = self.election_cycle_days.max(1);
        }
    }

    fn recompute_overall_approval(&mut self) {
        let weighted: f32 = self
            .factions
            .iter()
            .map(|f| f.electorate_share * f.approval)
            .sum();
        self.overall_approval = weighted.clamp(0.0, 100.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factions_react_divergently_to_lvt_ubi_shortages_and_pollution() {
        let mut manager = FactionManager::default();
        let baseline_inputs = PoliticsUpdateInputs {
            lvt_rate: 0.0,
            property_tax_burden: 0.05,
            income_and_vat_rate: 0.15,
            business_profit_tax_rate: 0.10,
            daily_ubi_per_adult: 0.0,
            base_subsistence_wage: 25.0,
            low_wage_shortage_ratio: 0.0,
            industrial_shortage_ratio: 0.0,
            average_pollution: 5.0,
            average_noise: 5.0,
            power_coverage: 1.0,
            property_owner_weight: 25.0,
            worker_youth_weight: 40.0,
            industrialist_weight: 15.0,
            environmentalist_weight: 20.0,
        };
        manager.evaluate_targets(&baseline_inputs);
        let po_base = manager.factions[FactionId::PropertyOwners.index()].target_approval;
        let wy_base = manager.factions[FactionId::WorkersAndYouth.index()].target_approval;
        let ind_base = manager.factions[FactionId::Industrialists.index()].target_approval;
        let env_base = manager.factions[FactionId::Environmentalists.index()].target_approval;

        // 1. Georgist LVT + UBI reform: PropertyOwners drop, WorkersAndYouth and Environmentalists rise.
        let georgist_inputs = PoliticsUpdateInputs {
            lvt_rate: 0.35,
            daily_ubi_per_adult: 20.0,
            ..baseline_inputs
        };
        manager.evaluate_targets(&georgist_inputs);
        assert!(manager.factions[FactionId::PropertyOwners.index()].target_approval < po_base);
        assert_eq!(
            manager.factions[FactionId::PropertyOwners.index()].dominant_issue,
            FactionIssue::LvtBurden
        );
        assert!(manager.factions[FactionId::WorkersAndYouth.index()].target_approval > wy_base);
        assert_eq!(
            manager.factions[FactionId::WorkersAndYouth.index()].dominant_issue,
            FactionIssue::UbiDividend
        );
        assert!(manager.factions[FactionId::Environmentalists.index()].target_approval > env_base);

        // 2. Severe labor shortage & pollution: Industrialists and Environmentalists drop.
        let crisis_inputs = PoliticsUpdateInputs {
            low_wage_shortage_ratio: 0.65,
            industrial_shortage_ratio: 0.50,
            average_pollution: 65.0,
            average_noise: 50.0,
            ..baseline_inputs
        };
        manager.evaluate_targets(&crisis_inputs);
        assert!(manager.factions[FactionId::Industrialists.index()].target_approval < ind_base);
        assert_eq!(
            manager.factions[FactionId::Industrialists.index()].dominant_issue,
            FactionIssue::LaborShortage
        );
        assert!(manager.factions[FactionId::Environmentalists.index()].target_approval < env_base);
        assert_eq!(
            manager.factions[FactionId::Environmentalists.index()].dominant_issue,
            FactionIssue::PollutionAndNoise
        );
    }

    #[test]
    fn election_cycle_advances_and_amplifies_campaign_volatility() {
        let mut manager = FactionManager {
            election_cycle_days: 5,
            days_until_election: 5,
            ..Default::default()
        };
        assert!(manager.current_campaign_volatility() > 1.5);
        manager.record_policy_change(0.5);
        assert!(manager.tax_reform_shock > 10.0);

        let inputs = PoliticsUpdateInputs {
            daily_ubi_per_adult: 25.0,
            base_subsistence_wage: 25.0,
            power_coverage: 1.0,
            ..Default::default()
        };
        for _ in 0..5 {
            manager.advance_day(&inputs);
        }
        assert_eq!(manager.elections_completed, 1);
        assert_eq!(manager.days_until_election, 5);
        assert!(manager.last_election_vote_share > 50.0);
        assert!(manager.last_election_won);
        assert_eq!(manager.elections_won, 1);
    }
}
