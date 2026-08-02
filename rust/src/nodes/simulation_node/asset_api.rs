//! Asset editor and asset registry Godot API methods.

use super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// Scans a native filesystem directory for content packs and registers all valid assets.
    #[func]
    pub fn load_asset_packs(&mut self, dir_path: GString, enabled_pack_ids: GString) -> GString {
        use crate::nodes::sim::bridge::assets::load_asset_packs;
        load_asset_packs(&mut self.lock_core(), dir_path, enabled_pack_ids)
    }

    /// Returns all qualified asset IDs (`"pack_id:asset_id"`) currently in the registry.
    ///
    /// Godot uses this to enumerate which meshes to load for building rendering.
    #[func]
    pub fn get_registered_asset_ids(&self) -> PackedStringArray {
        let core = self.lock_core();
        let mut ids: Vec<GString> = core
            .allocator
            .registry
            .qualified_ids()
            .map(GString::from)
            .collect();

        let has_broken = core.allocator.buildings.iter().any(|b| b.broken);
        if has_broken {
            ids.push(GString::from("broken:error"));
        }

        PackedStringArray::from_iter(ids)
    }

    /// Returns the number of renderable building mesh parts for a registered asset.
    #[func]
    pub fn get_building_mesh_part_count(&self, qualified_id: GString) -> i32 {
        use crate::nodes::sim::bridge::assets::get_building_mesh_part_count;
        get_building_mesh_part_count(&self.lock_core(), qualified_id)
    }

    /// Returns the native filesystem path to one building mesh part's LOD0 mesh file.
    #[func]
    pub fn get_building_mesh_part_lod0_native_path(
        &self,
        qualified_id: GString,
        part_index: i32,
    ) -> GString {
        use crate::nodes::sim::bridge::assets::get_building_mesh_part_lod0_native_path;
        get_building_mesh_part_lod0_native_path(&self.lock_core(), qualified_id, part_index)
    }

    /// Returns one coherent building-render frame, or `busy = true` without waiting for SimCore.
    ///
    /// Asset-part and zone outputs preserve the order of the supplied request arrays. Site mesh
    /// buffers are included only when their revision differs from `known_site_revision`.
    #[func]
    pub fn try_get_building_render_frame(
        &self,
        asset_ids: PackedStringArray,
        part_indices: PackedInt32Array,
        zone_ids: PackedInt32Array,
        known_site_revision: i64,
    ) -> VarDictionary {
        let mut frame = VarDictionary::new();
        frame.set("busy", true);
        let Some(core) = self.try_lock_core() else {
            return frame;
        };

        let part_count = asset_ids.len().min(part_indices.len());
        let mut building_transforms = VarArray::new();
        let mut deserted_transforms = VarArray::new();
        for index in 0..part_count {
            let asset_id = asset_ids[index].to_string();
            let part_index = part_indices[index];
            building_transforms.push(
                &core
                    .get_building_transforms_for_asset_part_internal(&asset_id, part_index)
                    .to_variant(),
            );
            deserted_transforms.push(
                &core
                    .get_deserted_building_transforms_for_asset_part_internal(&asset_id, part_index)
                    .to_variant(),
            );
        }

        let mut plot_transforms = VarArray::new();
        let mut construction_site_transforms = VarArray::new();
        let mut construction_foundation_transforms = VarArray::new();
        let mut construction_scaffold_transforms = VarArray::new();
        for &zone_id in zone_ids.as_slice() {
            let zone_id = u8::try_from(zone_id).unwrap_or(0);
            plot_transforms.push(
                &core
                    .get_building_plot_transforms_internal(zone_id)
                    .to_variant(),
            );
            construction_site_transforms.push(
                &core
                    .get_construction_site_transforms_internal(zone_id)
                    .to_variant(),
            );
            construction_foundation_transforms.push(
                &core
                    .get_construction_foundation_transforms_internal(zone_id)
                    .to_variant(),
            );
            construction_scaffold_transforms.push(
                &core
                    .get_construction_scaffold_transforms_internal(zone_id)
                    .to_variant(),
            );
        }

        let site_revision = core.get_building_site_revision_internal();
        frame.set("busy", false);
        frame.set("building_transforms", building_transforms.to_variant());
        frame.set("deserted_transforms", deserted_transforms.to_variant());
        frame.set("plot_transforms", plot_transforms.to_variant());
        frame.set(
            "construction_site_transforms",
            construction_site_transforms.to_variant(),
        );
        frame.set(
            "construction_foundation_transforms",
            construction_foundation_transforms.to_variant(),
        );
        frame.set(
            "construction_scaffold_transforms",
            construction_scaffold_transforms.to_variant(),
        );
        frame.set(
            "site_revision",
            i64::try_from(site_revision).unwrap_or(i64::MAX),
        );
        if known_site_revision < 0 || u64::try_from(known_site_revision).ok() != Some(site_revision)
        {
            frame.set(
                "site_mesh_data",
                core.get_building_site_mesh_data_internal().to_variant(),
            );
        }
        frame
    }

    /// Returns a Dictionary of live stats for the building whose centre is closest to
    /// (`world_x`, `world_z`) within a 30 m pick radius.
    ///
    /// Returns an empty Dictionary when no building is within range.
    /// Keys: `asset_id`, `zone_type`, `level`, `occupancy`, `worker_count`,
    /// `worker_capacity`, compact business summary fields, `budget_distress`,
    /// `economy_broken`, `broken`, `pending_redevelopment`, `rezone_grace_days`,
    /// `economy_profile`, `center_x`, `center_z`, residential household aggregates,
    /// extractor reserve fields, and `inventory` (Array of `{name, amount}` Dictionaries).
    #[func]
    pub fn get_building_info_at(&self, world_x: f32, world_z: f32) -> VarDictionary {
        use crate::simulation::economy::definitions::{
            EconomyProfileRuntimeKind, load_runtime_economy_catalog,
        };
        use crate::simulation::economy::households::{
            REPLENISHMENT_COOLDOWN, REPLENISHMENT_FAILED_TERMINAL, REPLENISHMENT_FULFILLED,
            REPLENISHMENT_NEEDS, REPLENISHMENT_SHOPPING_RETURNING, REPLENISHMENT_SHOPPING_TO_STORE,
            REPLENISHMENT_STABLE, REPLENISHMENT_WAITING_FOR_SHOPPER,
        };
        use crate::simulation::economy::households::{
            building_inventory_fill_ratio, building_operation_factors,
        };
        use crate::simulation::zoning::ZoneType;

        let core = self.lock_core();

        // Linear scan — only called on explicit user clicks, never on the hot path.
        let pick_radius_sq = 30.0_f32 * 30.0;
        let mut best_idx = usize::MAX;
        let mut best_dist_sq = pick_radius_sq;
        for (i, b) in core.allocator.buildings.iter().enumerate() {
            let dx = b.center_x - world_x;
            let dz = b.center_y - world_z; // center_y is world-Z in the building struct
            let dist_sq = dx * dx + dz * dz;
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                best_idx = i;
            }
        }
        if best_idx == usize::MAX {
            return VarDictionary::new();
        }

        let b = &core.allocator.buildings[best_idx];
        let catalog = load_runtime_economy_catalog().ok();

        let zone_type_str = match b.zone_type {
            ZoneType::None => "utility",
            ZoneType::Residential => "residential",
            ZoneType::Commercial => "commercial",
            ZoneType::Industrial => "industrial",
            ZoneType::Office => "office",
            ZoneType::Mixed => "mixed",
        };

        let profile_id = catalog
            .as_ref()
            .and_then(|c| c.profile_by_runtime_id(b.economy_profile_runtime_id))
            .map(|p| p.id.clone())
            .unwrap_or_default();

        let worker_capacity = catalog
            .as_ref()
            .map(|catalog| {
                core.allocator
                    .worker_capacity_with_catalog(best_idx, catalog.as_ref())
            })
            .unwrap_or_else(|| core.allocator.worker_capacity(best_idx));

        // Inventory: only non-zero resource slots.
        let mut inv_arr = VarArray::new();
        if let Some(cat) = &catalog {
            for (slot, &amount) in b.resource_inventory.iter().enumerate() {
                if amount > 0.001 {
                    let runtime_id = (slot + 1) as u16;
                    let name = cat
                        .resource_id_for_runtime_id(runtime_id)
                        .unwrap_or("unknown");
                    let mut entry = VarDictionary::new();
                    entry.set("name", GString::from(name));
                    entry.set("amount", amount as f64);
                    inv_arr.push(&entry.to_variant());
                }
            }
        }

        let mut dict = VarDictionary::new();
        dict.set("asset_id", GString::from(b.asset_id.as_str()));
        let asset_display_name = core
            .allocator
            .registry
            .get(&b.asset_id)
            .map(|entry| entry.manifest.display_name.as_str())
            .unwrap_or(b.asset_id.as_str());
        dict.set("asset_display_name", GString::from(asset_display_name));
        dict.set("zone_type", GString::from(zone_type_str));
        dict.set("level", b.level as i32);
        dict.set("under_construction", b.is_under_construction());
        dict.set(
            "construction_remaining_hours",
            b.construction_remaining_hours as i32,
        );
        dict.set("construction_progress", b.construction_progress() as f64);
        dict.set("occupancy", b.occupancy as i32);
        dict.set("center_x", b.center_x as f64);
        dict.set("center_z", b.center_y as f64);
        if let Some(resource_id) = core.allocator.registry.extractor_resource(&b.asset_id) {
            dict.set("extractor_resource", GString::from(resource_id));
            if let Some(site) = core.resource_extraction.site_for_building(best_idx) {
                let total = site.total_reserve_units.max(0.0);
                let consumed = site.extracted_units.clamp(0.0, total);
                let available = (total - consumed).max(0.0);
                let consumed_ratio = if total > 0.0 { consumed / total } else { 0.0 };
                dict.set("extractor_has_site", true);
                dict.set("extractor_total_reserve_units", total as f64);
                dict.set("extractor_available_reserve_units", available as f64);
                dict.set("extractor_consumed_reserve_units", consumed as f64);
                dict.set("extractor_consumed_reserve_ratio", consumed_ratio as f64);
            } else {
                dict.set("extractor_has_site", false);
                dict.set("extractor_total_reserve_units", 0.0f64);
                dict.set("extractor_available_reserve_units", 0.0f64);
                dict.set("extractor_consumed_reserve_units", 0.0f64);
                dict.set("extractor_consumed_reserve_ratio", 0.0f64);
            }
        }
        if let Some(resource_id) = core.allocator.registry.field_resource(&b.asset_id) {
            dict.set("field_resource", GString::from(resource_id));
            if let Some(site) = core.agriculture.site_for_building(best_idx) {
                dict.set("field_has_site", true);
                dict.set("field_area_m2", f64::from(site.area_m2.max(0.0)));
            } else {
                dict.set("field_has_site", false);
                dict.set("field_area_m2", 0.0f64);
            }
        }

        let mut total_agents = 0i32;
        let mut child_agents = 0i32;
        let mut adult_agents = 0i32;
        let mut elder_agents = 0i32;
        let mut household_count = 0i32;
        let mut household_budget_total = 0.0f32;
        let mut household_stock_total = 0.0f32;
        let mut household_stock_days_total = 0.0f32;
        let mut household_stock_days_min = f32::INFINITY;
        let mut household_replenishment_active = 0i32;
        let mut first_replenishment_state = None;
        let mut mixed_replenishment_state = false;
        if b.zone_type == ZoneType::Residential {
            for h in &core.households.households {
                if h.home_building_id == best_idx {
                    household_count += 1;
                    total_agents += h.member_count as i32;
                    child_agents += h.child_count as i32;
                    adult_agents += h.adult_count as i32;
                    elder_agents += h.elder_count as i32;
                    household_budget_total += h.budget;
                    household_stock_total += h.stock;
                    household_stock_days_total += h.stock_days;
                    household_stock_days_min = household_stock_days_min.min(h.stock_days);
                    if h.replenishment_state != REPLENISHMENT_STABLE {
                        household_replenishment_active += 1;
                    }
                    match first_replenishment_state {
                        Some(state) if state != h.replenishment_state => {
                            mixed_replenishment_state = true;
                        }
                        None => {
                            first_replenishment_state = Some(h.replenishment_state);
                        }
                        _ => {}
                    }
                }
            }
        }
        dict.set("agent_count", total_agents);
        dict.set("child_count", child_agents);
        dict.set("adult_count", adult_agents);
        dict.set("elder_count", elder_agents);
        if b.zone_type == ZoneType::Residential {
            let household_divisor = household_count.max(1) as f32;
            let replenishment_state = if household_count == 0 {
                "-"
            } else if mixed_replenishment_state {
                "Mixed"
            } else {
                match first_replenishment_state.unwrap_or(REPLENISHMENT_STABLE) {
                    REPLENISHMENT_STABLE => "Stable",
                    REPLENISHMENT_NEEDS => "Needs restock",
                    REPLENISHMENT_WAITING_FOR_SHOPPER => "Waiting for shopper",
                    REPLENISHMENT_SHOPPING_TO_STORE => "Shopping to store",
                    REPLENISHMENT_SHOPPING_RETURNING => "Shopping returning",
                    REPLENISHMENT_FULFILLED => "Fulfilled",
                    REPLENISHMENT_COOLDOWN => "Cooldown",
                    REPLENISHMENT_FAILED_TERMINAL => "Unresolved shortage",
                    _ => "Unknown",
                }
            };
            dict.set("household_count", household_count);
            dict.set("household_budget_total", household_budget_total as f64);
            dict.set(
                "household_budget_avg",
                (household_budget_total / household_divisor) as f64,
            );
            dict.set("household_stock_total", household_stock_total as f64);
            dict.set(
                "household_stock_days_avg",
                (household_stock_days_total / household_divisor) as f64,
            );
            dict.set(
                "household_stock_days_min",
                if household_stock_days_min.is_finite() {
                    household_stock_days_min as f64
                } else {
                    0.0
                },
            );
            dict.set(
                "household_replenishment_active",
                household_replenishment_active,
            );
            dict.set(
                "household_replenishment_state",
                GString::from(replenishment_state),
            );
        }
        dict.set("worker_count", b.worker_count as i32);
        dict.set("worker_capacity", worker_capacity as i32);
        dict.set("operating_budget", b.operating_budget as f64);
        dict.set("revenue", b.revenue as f64);
        if b.zone_type != ZoneType::Residential {
            if let Some(cat) = &catalog
                && let Some(profile) = cat.profile_by_runtime_id(b.economy_profile_runtime_id)
            {
                let factors = building_operation_factors(cat.as_ref(), b, profile);
                let inventory_fill = building_inventory_fill_ratio(cat.as_ref(), b, profile);
                let profit_today = b.operating_budget - b.profit_tax_budget_baseline;
                let business_status = if b.broken {
                    "Asset broken"
                } else if b.economy_broken {
                    "Economy broken"
                } else if b.is_deserted {
                    "Deserted"
                } else if b.is_under_construction() {
                    "Under construction"
                } else if b.budget_distress || b.operating_budget < 0.0 {
                    "Distressed"
                } else if factors.active_worker_capacity > 0 && factors.effective_workers == 0 {
                    "No workers"
                } else if factors.input_factor < 0.5 {
                    "Needs inputs"
                } else if factors.output_headroom_factor < 0.5 {
                    "Storage full"
                } else if factors.active_worker_capacity < profile.worker_capacity {
                    "Demand-limited"
                } else if factors.throughput_factor >= 0.8 {
                    "Running"
                } else if factors.throughput_factor > 0.0 {
                    "Limited"
                } else {
                    "Idle"
                };
                dict.set("business_summary", true);
                dict.set("business_status", GString::from(business_status));
                dict.set("business_profit_today", profit_today as f64);
                dict.set("business_profit_yesterday", b.last_day_profit as f64);
                dict.set(
                    "business_active_worker_capacity",
                    factors.active_worker_capacity as i32,
                );
                dict.set(
                    "business_production_ratio",
                    factors.throughput_factor as f64,
                );
                dict.set(
                    "business_inventory_fill_ratio",
                    inventory_fill.unwrap_or(0.0) as f64,
                );
                dict.set("business_has_inventory_fill", inventory_fill.is_some());
                if matches!(
                    profile.kind,
                    EconomyProfileRuntimeKind::UtilityProducer
                        | EconomyProfileRuntimeKind::UtilityProcessor
                ) {
                    let utility_service = profile.utility_service.as_deref().unwrap_or("");
                    let utility_active = !b.broken
                        && !b.economy_broken
                        && !b.is_deserted
                        && !b.is_under_construction()
                        && b.edge_idx != usize::MAX
                        && b.worker_count > 0
                        && factors.throughput_factor > 0.0;
                    dict.set("utility_service", GString::from(utility_service));
                    dict.set("utility_service_available", utility_active);
                    dict.set("utility_local_revenue", b.revenue as f64);
                    if utility_service == "power" {
                        dict.set(
                            "service_funding_effective",
                            core.effective_electricity_funding_for_building(best_idx) as f64,
                        );
                        dict.set(
                            "service_funding_city",
                            core.service_policy.electricity_funding as f64,
                        );
                        dict.set(
                            "service_funding_override",
                            b.service_funding_override >= 0.0,
                        );
                    }
                    if let Some(fuel_port) = profile.inputs.first() {
                        let fuel_name = cat
                            .resource_id_for_runtime_id(fuel_port.resource_runtime_id)
                            .unwrap_or("fuel");
                        let fuel_units = b.inventory_units(fuel_port.resource_runtime_id).max(0.0);
                        let fuel_days = if fuel_port.units_per_day > 0.0 {
                            fuel_units / fuel_port.units_per_day
                        } else {
                            0.0
                        };
                        dict.set("utility_fuel_name", GString::from(fuel_name));
                        dict.set("utility_fuel_units", fuel_units as f64);
                        dict.set("utility_fuel_days", fuel_days as f64);
                    }
                    let power_production = b.recent_power_service_units.max(0.0);
                    let power_consumed = b.recent_power_served_units.clamp(0.0, power_production);
                    let power_unused = (power_production - power_consumed).max(0.0);
                    let power_consumption_ratio = if power_production > 0.0 {
                        power_consumed / power_production
                    } else {
                        0.0
                    };
                    dict.set("utility_power_production_today", power_production as f64);
                    dict.set("utility_power_consumed_today", power_consumed as f64);
                    dict.set("utility_power_unused_today", power_unused as f64);
                    dict.set(
                        "utility_power_consumption_ratio",
                        power_consumption_ratio as f64,
                    );
                    dict.set(
                        "city_fuel_cost_today",
                        b.daily_city_funded_input_cost as f64,
                    );
                }
            }
        }
        dict.set("budget_distress", b.budget_distress);
        dict.set("economy_broken", b.economy_broken);
        dict.set("broken", b.broken);
        dict.set("is_deserted", b.is_deserted);
        dict.set("pending_redevelopment", b.pending_redevelopment);
        dict.set("rezone_grace_days", b.rezone_grace_days_remaining as i32);
        dict.set("economy_profile", GString::from(profile_id.as_str()));
        dict.set("inventory", inv_arr.to_variant());
        dict
    }

    /// Validates the JSON export params, writes `pack.toml` (if absent) and
    /// `assets/<asset_id>/asset.toml` under `output_dir`, and returns an error
    /// string or `""` on success.
    ///
    /// `output_dir` must be an absolute native path (resolve `user://mods/<pack_id>/`
    /// with `ProjectSettings.globalize_path` before passing it in).
    #[func]
    pub fn validate_and_export_asset(&self, params_json: GString, output_dir: GString) -> GString {
        use crate::nodes::sim::asset_export::validate_and_export_asset_internal;
        let result =
            validate_and_export_asset_internal(&params_json.to_string(), &output_dir.to_string());
        GString::from(result.as_str())
    }

    /// Returns a JSON object describing the manifest for an already-registered asset,
    /// or `""` if the qualified ID is not in the registry.
    ///
    /// GDScript uses this to repopulate the importer form when re-editing an existing asset.
    #[func]
    pub fn get_asset_manifest_json(&self, qualified_id: GString) -> GString {
        use crate::nodes::sim::asset_export::get_asset_manifest_json_internal;
        let core = self.lock_core();
        let result =
            get_asset_manifest_json_internal(&core.allocator.registry, &qualified_id.to_string());
        GString::from(result.as_str())
    }

    /// Returns a JSON object with pack metadata (`pack_id`, `display_name`, `author`,
    /// `version`, `license`) read from `<output_dir>/pack.toml`, or `""` if not found.
    ///
    /// `output_dir` must be the absolute native path to the pack directory
    /// (i.e. `ProjectSettings.globalize_path("user://mods/<pack_id>/")` ).
    #[func]
    pub fn get_pack_manifest_json(&self, output_dir: GString) -> GString {
        use crate::nodes::sim::asset_export::get_pack_manifest_json_internal;
        GString::from(get_pack_manifest_json_internal(&output_dir.to_string()).as_str())
    }
}
