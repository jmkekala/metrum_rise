// SPDX-License-Identifier: GPL-2.0-only

//! Economy and demand Godot API methods.

use super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// Returns cached hourly R/C/I construction opportunities and their concrete constraints.
    #[func]
    pub fn get_growth_overview(&self) -> VarDictionary {
        let core = self.lock_core();
        let mut result = VarDictionary::new();
        for (key, (projects, detail)) in ["residential", "commercial", "industrial"]
            .into_iter()
            .zip(core.demand.growth_display())
        {
            let mut entry = VarDictionary::new();
            entry.set("projects", projects as i64);
            entry.set("detail", GString::from(&detail));
            result.set(key, entry);
        }
        result
    }

    /// Grants cheat money and bypasses economic entry checks for private construction.
    #[func]
    pub fn apply_money_and_max_demand_cheat(&mut self) -> f64 {
        let balance = {
            let mut core = self.lock_core();
            core.apply_money_and_max_demand_cheat(CHEAT_MONEY_GRANT_AMOUNT)
        };
        self.refresh_snapshot_from_core();
        balance
    }

    /// Returns city budget history, service policy, and compact service status for UI windows.
    #[func]
    pub fn get_economy_overview(&self) -> VarDictionary {
        let core = self.lock_core();
        let mut history = VarArray::new();
        for entry in &core.budget_history {
            let dict = budget_ledger_entry_dict(entry);
            history.push(&dict.to_variant());
        }

        let latest =
            core.budget_history
                .back()
                .copied()
                .unwrap_or_else(|| DailyBudgetLedgerEntry {
                    day_index: core.time.day_index,
                    treasury: core.treasury.balance,
                    ..Default::default()
                });

        let mut services = VarArray::new();
        let mut electricity = VarDictionary::new();
        electricity.set("id", GString::from(SERVICE_POLICY_ELECTRICITY));
        electricity.set("name", GString::from("Electricity"));
        electricity.set("funding", core.service_policy.electricity_funding as f64);
        electricity.set("coverage", latest.power_coverage);
        electricity.set("produced", latest.power_produced);
        electricity.set("consumed", latest.power_consumed);
        electricity.set("unmet", latest.power_unmet);
        electricity.set(
            "active",
            latest.power_produced > 0.0 && latest.power_consumed > 0.0,
        );
        let status = if latest.power_unmet > 0.01 {
            "shortage"
        } else if latest.power_produced > 0.01 {
            "stable"
        } else {
            "inactive"
        };
        electricity.set("status", GString::from(status));
        services.push(&electricity.to_variant());

        let mut policy = VarDictionary::new();
        policy.set(
            SERVICE_POLICY_ELECTRICITY,
            core.service_policy.electricity_funding as f64,
        );

        let mut dict = VarDictionary::new();
        dict.set("current_day", core.time.day_index as i32);
        dict.set("treasury", core.treasury.balance);
        dict.set("history", history);
        dict.set("latest", budget_ledger_entry_dict(&latest));
        dict.set("policy", policy);
        dict.set("fiscal_policy", fiscal_policy_dict(core.fiscal_policy));
        dict.set(
            "fiscal_policy_controls",
            fiscal_policy_controls_array(core.fiscal_policy),
        );
        dict.set(
            "city_budget",
            city_budget_dict(
                &core.treasury.budget,
                core.land_value.total_assessed_land_value,
            ),
        );
        dict.set(
            "labor_market",
            labor_market_dict(&core.households.labor_market),
        );
        dict.set("politics", politics_dict(&core.treasury.budget.politics));
        dict.set("services", services);
        dict
    }

    /// Returns political factions, approval ratings, dominant issues, and election countdown.
    #[func]
    pub fn get_politics_overview(&self) -> VarDictionary {
        let core = self.lock_core();
        politics_dict(&core.treasury.budget.politics)
    }

    /// Computes a non-mutating real-time preview of faction approval deltas, projected UBI
    /// payouts, and reservation-wage labor shortage warnings for a dictionary of staged policy
    /// values (`{ policy_id: value }`).
    #[func]
    pub fn preview_policy_reform(&self, proposed_policies: VarDictionary) -> VarDictionary {
        let changes = parse_policy_changes_dict(&proposed_policies);
        let core = self.lock_core();
        let preview = core.preview_policy_reform(&changes);
        policy_reform_preview_dict(&preview)
    }

    /// Atomically commits a dictionary of staged policy values (`{ policy_id: value }`) and
    /// returns the updated economy overview dictionary.
    #[func]
    pub fn apply_policy_reform(&mut self, proposed_policies: VarDictionary) -> VarDictionary {
        let changes = parse_policy_changes_dict(&proposed_policies);
        {
            let mut core = self.lock_core();
            core.apply_policy_reform(&changes);
        }
        self.get_economy_overview()
    }

    /// Applies a live city service funding value. Returns `false` for unknown service ids.
    #[func]
    pub fn set_economy_service_funding(&mut self, service_id: GString, funding: f32) -> bool {
        let mut core = self.lock_core();
        core.set_service_funding(&service_id.to_string(), funding)
    }

    /// Applies a live fiscal policy value. Returns `false` for unknown policy ids.
    #[func]
    pub fn set_economy_policy_value(&mut self, policy_id: GString, value: f32) -> bool {
        let mut core = self.lock_core();
        core.set_fiscal_policy_value(&policy_id.to_string(), value)
    }

    /// Applies a live service funding override to the service building nearest the world point.
    #[func]
    pub fn set_building_service_funding_override_at(
        &mut self,
        world_x: f32,
        world_z: f32,
        service_id: GString,
        funding: f32,
    ) -> bool {
        let mut core = self.lock_core();
        core.set_building_service_funding_override_at(
            world_x,
            world_z,
            &service_id.to_string(),
            funding,
        )
    }
}

fn fiscal_policy_dict(
    policy: crate::simulation::economy::fiscal::CityFiscalPolicy,
) -> VarDictionary {
    let mut dict = VarDictionary::new();
    for control in policy.controls() {
        dict.set(control.id, control.value as f64);
    }
    dict
}

fn fiscal_policy_controls_array(
    policy: crate::simulation::economy::fiscal::CityFiscalPolicy,
) -> VarArray {
    let mut controls = VarArray::new();
    for control in policy.controls() {
        let mut dict = VarDictionary::new();
        dict.set("id", GString::from(control.id));
        dict.set("label", GString::from(control.label));
        dict.set("group", GString::from(control.group));
        dict.set("unit", GString::from(control.unit.as_str()));
        dict.set("value", control.value as f64);
        dict.set("min", control.min as f64);
        dict.set("max", control.max as f64);
        dict.set("step", control.step as f64);
        let impact = fiscal_policy_control_impact(control.id);
        dict.set("impact_field", GString::from(impact.0));
        dict.set("impact_label", GString::from(impact.1));
        dict.set("impact_kind", GString::from(impact.2));
        controls.push(&dict.to_variant());
    }
    controls
}

fn city_budget_dict(
    budget: &crate::nodes::sim::core::CityBudget,
    total_assessed_land_value: f64,
) -> VarDictionary {
    let mut dict = VarDictionary::new();
    dict.set("debt_principal", budget.debt_principal);
    dict.set("daily_debt_interest_rate", budget.daily_debt_interest_rate);
    dict.set("last_day_gross_revenue", budget.last_day_gross_revenue);
    dict.set("last_day_lvt_revenue", budget.last_day_lvt_revenue);
    dict.set("last_day_mandatory_opex", budget.last_day_mandatory_opex);
    dict.set("last_day_debt_service", budget.last_day_debt_service);
    dict.set("last_day_pre_ubi_surplus", budget.last_day_pre_ubi_surplus);
    dict.set("active_ubi_pool", budget.active_ubi_pool);
    dict.set("remaining_ubi_pool", budget.remaining_ubi_pool);
    dict.set("daily_ubi_per_adult", f64::from(budget.daily_ubi_per_adult));
    dict.set("last_day_ubi_paid", budget.last_day_ubi_paid);
    dict.set("total_assessed_land_value", total_assessed_land_value);
    dict
}

fn labor_market_dict(
    lm: &crate::simulation::economy::labor_market::LaborMarket,
) -> VarDictionary {
    let mut dict = VarDictionary::new();
    dict.set("base_subsistence_wage", f64::from(lm.base_subsistence_wage));
    dict.set("reservation_wage", f64::from(lm.reservation_wage));
    dict.set(
        "ubi_reservation_elasticity",
        f64::from(lm.ubi_reservation_elasticity),
    );
    dict.set(
        "public_sector_wage_multiplier",
        f64::from(lm.public_sector_wage_multiplier),
    );
    dict.set("automation_level", f64::from(lm.automation_level));
    dict.set(
        "active_daily_ubi_per_adult",
        f64::from(lm.active_daily_ubi_per_adult),
    );
    let sector_names = [
        "low_wage_commercial",
        "industrial",
        "skilled_private",
        "public_sector",
    ];
    let mut sectors = VarDictionary::new();
    for (idx, name) in sector_names.into_iter().enumerate() {
        let mut s = VarDictionary::new();
        s.set("funded_jobs", lm.funded_jobs_by_sector[idx] as i64);
        s.set("filled_jobs", lm.filled_jobs_by_sector[idx] as i64);
        s.set("open_vacancies", lm.open_vacancies_by_sector[idx] as i64);
        s.set(
            "reservation_wage_refusals",
            lm.reservation_wage_refusals_by_sector[idx] as i64,
        );
        s.set("shortage_ratio", f64::from(lm.sector_shortage_ratio[idx]));
        sectors.set(name, s);
    }
    dict.set("sectors", sectors);
    dict
}

fn politics_dict(
    politics: &crate::simulation::economy::politics::FactionManager,
) -> VarDictionary {
    use crate::simulation::economy::politics::FactionId;

    let mut dict = VarDictionary::new();
    dict.set("election_cycle_days", politics.election_cycle_days as i64);
    dict.set("days_until_election", politics.days_until_election as i64);
    dict.set("overall_approval", f64::from(politics.overall_approval));
    dict.set("elections_completed", politics.elections_completed as i64);
    dict.set("elections_won", politics.elections_won as i64);
    dict.set(
        "last_election_vote_share",
        f64::from(politics.last_election_vote_share),
    );
    dict.set("last_election_won", politics.last_election_won);
    dict.set("tax_reform_shock", f64::from(politics.tax_reform_shock));
    dict.set(
        "campaign_volatility",
        f64::from(politics.current_campaign_volatility()),
    );

    let mut factions = VarArray::new();
    for id in FactionId::ALL {
        let state = &politics.factions[id.index()];
        let mut f = VarDictionary::new();
        f.set("id", GString::from(id.id_str()));
        f.set("name", GString::from(id.display_name()));
        f.set("electorate_share", f64::from(state.electorate_share));
        f.set("target_approval", f64::from(state.target_approval));
        f.set("approval", f64::from(state.approval));
        f.set("campaign_volatility", f64::from(state.campaign_volatility));
        f.set("dominant_issue", GString::from(state.dominant_issue.as_str()));
        factions.push(&f.to_variant());
    }
    dict.set("factions", factions);
    dict
}

fn parse_policy_changes_dict(proposed_policies: &VarDictionary) -> Vec<(String, f32)> {
    let mut changes = Vec::with_capacity(proposed_policies.len());
    for (k, v) in proposed_policies.iter_shared() {
        let Ok(key) = k.try_to::<GString>() else {
            continue;
        };
        let value = if let Ok(f) = v.try_to::<f64>() {
            f as f32
        } else if let Ok(i) = v.try_to::<i64>() {
            i as f32
        } else {
            continue;
        };
        if value.is_finite() {
            changes.push((key.to_string(), value));
        }
    }
    changes
}

fn policy_reform_preview_dict(
    preview: &crate::nodes::sim::core::PolicyReformPreview,
) -> VarDictionary {
    use crate::simulation::economy::politics::FactionId;

    let mut dict = VarDictionary::new();
    dict.set("projected_lvt_revenue", preview.projected_lvt_revenue);
    dict.set(
        "projected_pre_ubi_surplus",
        preview.projected_pre_ubi_surplus,
    );
    dict.set(
        "projected_daily_ubi_per_adult",
        f64::from(preview.projected_daily_ubi_per_adult),
    );
    dict.set(
        "deficit_circuit_breaker_active",
        preview.deficit_circuit_breaker_active,
    );
    dict.set(
        "projected_reservation_wage",
        f64::from(preview.projected_reservation_wage),
    );
    dict.set(
        "binding_reservation_wage",
        f64::from(preview.binding_reservation_wage),
    );
    dict.set(
        "reference_low_wage_net",
        f64::from(preview.reference_low_wage_net),
    );
    dict.set(
        "reference_public_sector_net",
        f64::from(preview.reference_public_sector_net),
    );
    dict.set(
        "low_wage_shortage_warning",
        preview.low_wage_shortage_warning,
    );
    dict.set(
        "public_sector_wage_warning",
        preview.public_sector_wage_warning,
    );
    dict.set(
        "projected_reform_shock",
        f64::from(preview.projected_reform_shock),
    );
    dict.set(
        "baseline_overall_approval",
        f64::from(preview.baseline_overall_approval),
    );
    dict.set(
        "preview_overall_approval",
        f64::from(preview.preview_overall_approval),
    );
    dict.set(
        "overall_delta",
        f64::from(preview.preview_overall_approval - preview.baseline_overall_approval),
    );

    let mut factions = VarArray::new();
    for id in FactionId::ALL {
        let idx = id.index();
        let base = &preview.baseline_factions[idx];
        let next = &preview.preview_factions[idx];
        let delta = next.target_approval - base.target_approval;
        let mut f = VarDictionary::new();
        f.set("id", GString::from(id.id_str()));
        f.set("name", GString::from(id.display_name()));
        f.set("electorate_share", f64::from(next.electorate_share));
        f.set("current_approval", f64::from(base.approval));
        f.set("baseline_target_approval", f64::from(base.target_approval));
        f.set("preview_target_approval", f64::from(next.target_approval));
        f.set("approval_delta", f64::from(delta));
        f.set(
            "dominant_issue",
            GString::from(next.dominant_issue.as_str()),
        );
        factions.push(&f.to_variant());
    }
    dict.set("factions", factions);
    dict
}

fn fiscal_policy_control_impact(policy_id: &str) -> (&'static str, &'static str, &'static str) {
    use crate::simulation::economy::fiscal::{
        POLICY_AUTOMATION_LEVEL, POLICY_BUSINESS_PROFIT_TAX, POLICY_CHILD_SUPPORT,
        POLICY_COMMERCIAL_PROPERTY_TAX, POLICY_HOUSEHOLD_VAT, POLICY_INCOME_TAX,
        POLICY_INDUSTRIAL_PROPERTY_TAX, POLICY_LVT_RATE, POLICY_PENSION,
        POLICY_PROPERTY_TAX_LEVEL_MULTIPLIER, POLICY_PUBLIC_WAGE_MULTIPLIER,
        POLICY_RESIDENTIAL_PROPERTY_TAX, POLICY_UBI_SURPLUS_SHARE, POLICY_UNEMPLOYMENT_BENEFIT,
        POLICY_UNEMPLOYMENT_MAX_DAYS,
    };

    match policy_id {
        POLICY_UNEMPLOYMENT_BENEFIT | POLICY_UNEMPLOYMENT_MAX_DAYS => {
            ("unemployment_benefits", "Unemployment", "expense")
        }
        POLICY_PENSION => ("pensions", "Pensions", "expense"),
        POLICY_CHILD_SUPPORT => ("child_support", "Child Support", "expense"),
        POLICY_INCOME_TAX => ("income_tax", "Income Tax", "revenue"),
        POLICY_HOUSEHOLD_VAT => ("household_vat", "Household VAT", "revenue"),
        POLICY_BUSINESS_PROFIT_TAX => ("business_profit_tax", "Business Profit Tax", "revenue"),
        POLICY_RESIDENTIAL_PROPERTY_TAX => (
            "residential_property_tax",
            "Residential Property Tax",
            "revenue",
        ),
        POLICY_COMMERCIAL_PROPERTY_TAX => (
            "commercial_property_tax",
            "Commercial Property Tax",
            "revenue",
        ),
        POLICY_INDUSTRIAL_PROPERTY_TAX => (
            "industrial_property_tax",
            "Industrial Property Tax",
            "revenue",
        ),
        POLICY_PROPERTY_TAX_LEVEL_MULTIPLIER => ("property_tax", "Property Tax", "revenue"),
        POLICY_LVT_RATE => ("lvt_tax", "Land Value Tax", "revenue"),
        POLICY_UBI_SURPLUS_SHARE => ("ubi_dividends", "Citizen's Dividend (UBI)", "expense"),
        POLICY_PUBLIC_WAGE_MULTIPLIER => ("city_wages", "Public Sector Wages", "expense"),
        POLICY_AUTOMATION_LEVEL => ("net", "Workplace Automation", "revenue"),
        _ => ("net", "Policy Impact", "revenue"),
    }
}
