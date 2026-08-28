// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: economy_api.rs
//  script_path: rust/src/nodes/simulation_node/economy_api.rs
//  module_name: economy_api
//  version: 0.1.0
//  description: Economy and demand Godot API methods.
//  kind: module
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: []
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! Economy and demand Godot API methods.

use super::*;

// ========================================================================
// ECONOMY API
// ========================================================================

#[godot_api(secondary)]
impl SimulationNode {
    /// Returns normalized residential, commercial, and industrial demand pressures.
    #[func]
    pub fn get_demand_pressures(&self) -> Vector3 {
        let core = self.lock_core();
        Vector3::new(
            core.demand.net_residential_pressure().clamp(-1.0, 1.0),
            core.demand.net_commercial_pressure().clamp(-1.0, 1.0),
            core.demand.net_industrial_pressure().clamp(-1.0, 1.0),
        )
    }

    /// Grants cheat money and permanently pins R/C/I demand display and planning pressure to 100%.
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

        let latest = core.budget_history.back().copied().unwrap_or_else(|| {
            let mut entry = DailyBudgetLedgerEntry::default();
            entry.day_index = core.time.day_index;
            entry.treasury = core.treasury.balance;
            entry
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
        dict.set("services", services);
        dict
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

    // ── Economy Editor ──

    /// Loads the canonical authored economy folder and returns a JSON envelope
    /// containing profiles, controllers, scenarios, and validation messages.
    #[func]
    pub fn load_economy_project(&self, dir_path: GString) -> GString {
        use crate::simulation::economy::definitions::load_project_json;
        match load_project_json(std::path::Path::new(&dir_path.to_string())) {
            Ok(json) => GString::from(json.as_str()),
            Err(err) => {
                let payload = serde_json::json!({
                    "ok": false,
                    "error": err,
                    "validation": [],
                });
                GString::from(payload.to_string().as_str())
            }
        }
    }

    /// Validates the authored economy JSON payload, writes the canonical TOML
    /// files, and rebuilds the derived `economy.index.bin` cache.
    #[func]
    pub fn export_economy_project(&self, project_json: GString, dir_path: GString) -> GString {
        use crate::simulation::economy::definitions::export_project_json;
        match export_project_json(
            &project_json.to_string(),
            std::path::Path::new(&dir_path.to_string()),
        ) {
            Ok(json) => GString::from(json.as_str()),
            Err(err) => {
                let payload = serde_json::json!({
                    "ok": false,
                    "error": err,
                    "validation": [],
                });
                GString::from(payload.to_string().as_str())
            }
        }
    }

    /// Runs the small authored-economy sandbox for the selected scenario and
    /// returns daily series data plus summary bottleneck metrics as JSON.
    #[func]
    pub fn run_economy_sandbox(&self, project_json: GString, scenario_id: GString) -> GString {
        use crate::simulation::economy::definitions::run_sandbox_json;
        match run_sandbox_json(&project_json.to_string(), &scenario_id.to_string()) {
            Ok(json) => GString::from(json.as_str()),
            Err(err) => {
                let payload = serde_json::json!({
                    "ok": false,
                    "error": err,
                    "validation": [],
                });
                GString::from(payload.to_string().as_str())
            }
        }
    }
}

// ========================================================================
// POLICY MARSHALLING
// ========================================================================

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

fn fiscal_policy_control_impact(policy_id: &str) -> (&'static str, &'static str, &'static str) {
    use crate::simulation::economy::fiscal::{
        POLICY_BORDER_OPENNESS, POLICY_BUSINESS_PROFIT_TAX, POLICY_CHILD_SUPPORT,
        POLICY_COMMERCIAL_PROPERTY_TAX, POLICY_HOUSEHOLD_VAT, POLICY_INCOME_TAX,
        POLICY_INDUSTRIAL_PROPERTY_TAX, POLICY_PENSION, POLICY_PROPERTY_TAX_LEVEL_MULTIPLIER,
        POLICY_RESIDENTIAL_PROPERTY_TAX, POLICY_UNEMPLOYMENT_BENEFIT,
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
        // Border policy is not a tax. It changes who arrives, which reaches the
        // treasury only through the households that do or do not turn up, so it
        // reports against population rather than against a revenue line.
        POLICY_BORDER_OPENNESS => ("border_openness", "Border Openness", "population"),
        _ => ("net", "Policy Impact", "revenue"),
    }
}
