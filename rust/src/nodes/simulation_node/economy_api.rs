//! Economy and demand Godot API methods.

use super::*;

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
        dict.set("services", services);
        dict
    }

    /// Applies a live city service funding value. Returns `false` for unknown service ids.
    #[func]
    pub fn set_economy_service_funding(&mut self, service_id: GString, funding: f32) -> bool {
        let mut core = self.lock_core();
        core.set_service_funding(&service_id.to_string(), funding)
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
