// SPDX-License-Identifier: GPL-3.0-or-later

//! Asset editor and asset registry Godot API methods.

use super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// Replaces the registry with the selected packs; an empty selection disables every pack.
    #[func]
    pub fn load_asset_packs(
        &mut self,
        dir_path: GString,
        enabled_pack_ids: PackedStringArray,
    ) -> GString {
        use crate::nodes::sim::bridge::assets::load_asset_packs;
        load_asset_packs(&mut self.lock_core(), dir_path, Some(&enabled_pack_ids))
    }

    /// Refreshes all installed packs for authoring, independently of the gameplay selection.
    #[func]
    pub fn load_all_asset_packs(&mut self, dir_path: GString) -> GString {
        use crate::nodes::sim::bridge::assets::load_asset_packs;
        load_asset_packs(&mut self.lock_core(), dir_path, None)
    }

    /// Whether the selected packs in `dir_path` hold a building for any zoning profile, by the
    /// spawn filter a game uses. Static, for menus without a simulation: it scans the pack
    /// manifests on each call, so call it when a dialog opens, not per frame.
    #[func]
    pub fn enabled_packs_stock_zones(
        dir_path: GString,
        enabled_pack_ids: PackedStringArray,
    ) -> bool {
        crate::nodes::sim::bridge::assets::enabled_packs_stock_zones(dir_path, &enabled_pack_ids)
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

    /// The construction sites' rows from the per-frame publication (`RENDER-30`, `CODE-26`):
    /// never the core lock. Empty while the published generation equals `known_generation`;
    /// else the generation followed by the nine row buffers (zone-major: residential,
    /// commercial, industrial; each site pad, foundation, scaffold), `null` for a buffer whose
    /// rows are not newer than `known_generation`. `O(1)` unchanged, else `O(changed rows)`.
    #[func]
    pub fn get_building_site_rows(&self, known_generation: i64) -> VarArray {
        let rows = Arc::clone(&self.snapshot.read().unwrap().queries.site_rows);
        let mut result = VarArray::new();
        if rows.generation as i64 == known_generation {
            return result;
        }
        result.push(&(rows.generation as i64).to_variant());
        for buffer in &rows.buffers {
            if buffer.generation as i64 > known_generation {
                result.push(&PackedFloat32Array::from(&buffer.rows[..]).to_variant());
            } else {
                result.push(&Variant::nil());
            }
        }
        result
    }

    /// The publisher's perf-channel counters (`RENDER-30`): `site_refreshes` (refreshes after a
    /// moved building revision), `site_refresh_us` (their time) and `site_full_passes` (the
    /// roster's scans), each since the process started. Never the core lock.
    #[func]
    pub fn get_building_site_row_work(&self) -> VarDictionary {
        let (refreshes, refresh_us, full_passes) =
            crate::nodes::sim::render::building_sites::site_row_work();
        let mut out = VarDictionary::new();
        out.set("site_refreshes", refreshes as i64);
        out.set("site_refresh_us", refresh_us);
        out.set("site_full_passes", full_passes as i64);
        out
    }

    /// The lot center `Vector2(x, z)` of the building whose footprint holds the point (a direct
    /// hit: a click on a road beside a building picks nothing), or `null`. Pass the center to
    /// [`Self::get_building_info_at`] for its details.
    #[func]
    pub fn pick_building_at(&self, world_x: f32, world_z: f32) -> Variant {
        let mut core = self.lock_core();
        let cell_m = core.config.zone_cell_m;
        core.allocator
            .building_idx_containing(world_x, world_z, cell_m)
            .map_or_else(Variant::nil, |idx| {
                let b = &core.allocator.buildings[idx];
                Vector2::new(b.center_x, b.center_y).to_variant()
            })
    }

    /// Returns a Dictionary of live stats for the building whose centre is closest to
    /// (`world_x`, `world_z`) within a 30 m pick radius.
    ///
    /// Returns an empty Dictionary when no building is within range. A click picks with
    /// [`Self::pick_building_at`] first: this lookup takes the nearest lot center, which only
    /// an exact center (the inspector's refresh and edits) answers unambiguously.
    /// Keys: `asset_id`, `zone_type`, `level`, `occupancy`, `worker_count`,
    /// `worker_capacity`, compact business summary fields, `budget_distress`,
    /// `economy_broken`, `broken`, `pending_redevelopment`, `rezone_grace_days`,
    /// `economy_profile`, `center_x`, `center_z`, residential/farm household aggregates,
    /// extractor reserve fields, `footprint` (the building's outline as world points just above
    /// its base, the bulldoze highlight's), and `inventory` (Array of `{name, amount}`
    /// Dictionaries).
    #[func]
    pub fn get_building_info_at(&self, world_x: f32, world_z: f32) -> VarDictionary {
        use crate::simulation::economy::definitions::EconomyProfileRuntimeKind;
        use crate::simulation::economy::households::{
            REPLENISHMENT_COOLDOWN, REPLENISHMENT_FAILED_TERMINAL, REPLENISHMENT_FULFILLED,
            REPLENISHMENT_NEEDS, REPLENISHMENT_SHOPPING_RETURNING, REPLENISHMENT_SHOPPING_TO_STORE,
            REPLENISHMENT_STABLE, REPLENISHMENT_WAITING_FOR_SHOPPER, building_inventory_fill_ratio,
            building_operation_factors,
        };
        use crate::simulation::work_area::profile_kind_uses_explicit_work_area;
        use crate::simulation::zoning::ZoneType;

        let mut core = self.lock_core();
        let cell_m = core.config.zone_cell_m;
        let Some(best_idx) = core
            .allocator
            .nearest_building_idx_at(world_x, world_z, cell_m)
        else {
            return VarDictionary::new();
        };

        let b = &core.allocator.buildings[best_idx];
        let catalog = core.demand.runtime_catalog();

        let zone_type_str = match b.zone_type {
            ZoneType::None => "utility",
            ZoneType::Residential => "residential",
            ZoneType::Commercial => "commercial",
            ZoneType::Industrial => "industrial",
            ZoneType::Office => "office",
            ZoneType::Mixed => "mixed",
        };

        let profile = catalog.profile_by_runtime_id(b.economy_profile_runtime_id);
        let profile_id = profile.map(|p| p.id.as_str()).unwrap_or_default();
        let worker_capacity = core
            .allocator
            .worker_capacity_with_catalog(best_idx, catalog);

        // Keep empty input buffers visible so shortages can be inspected.
        let mut inv_arr = VarArray::new();
        for (slot, &amount) in b.resource_inventory.iter().enumerate() {
            let runtime_id = (slot + 1) as u16;
            let input = profile.and_then(|profile| profile.input_port(runtime_id));
            if amount > 0.001 || input.is_some() {
                let name = catalog
                    .resource_id_for_runtime_id(runtime_id)
                    .unwrap_or("unknown");
                let mut entry = VarDictionary::new();
                entry.set("name", GString::from(name));
                entry.set("amount", amount as f64);
                if let Some(measure) = catalog.resource_measure(runtime_id) {
                    entry.set("unit", GString::from(measure.unit.symbol()));
                }
                if let Some(input) = input {
                    let scale = if profile.is_some_and(|profile| {
                        crate::simulation::work_area::profile_kind_uses_explicit_work_area(
                            profile.kind,
                        )
                    }) {
                        b.work_area_scale
                    } else {
                        1.0
                    };
                    entry.set("daily_input_units", (input.units_per_day * scale) as f64);
                }
                inv_arr.push(&entry.to_variant());
            }
        }

        let mut dict = VarDictionary::new();
        dict.set("asset_id", GString::from(b.asset_id.as_str()));
        let handle = core.allocator.building_handles.handle(best_idx).to_bits();
        dict.set("parking_valid_places", core.parking.building_place_count(handle) as i64);
        let rejected = core.parking_rejections(best_idx);
        dict.set("parking_rejected", PackedStringArray::from_iter(rejected.iter().map(|s| GString::from(s.as_str()))));
        // Docks beside the parking places, only for an asset with a freight Driveway (`PARK-19`).
        if let Some((usable, bays, reasons)) = core.dock_lines(best_idx) {
            dict.set("dock_usable", usable as i64);
            dict.set("dock_bays", bays as i64);
            dict.set("dock_rejected", PackedStringArray::from_iter(reasons.iter().map(|s| GString::from(s.as_str()))));
            let drives = core.dock_drive_lines(best_idx);
            dict.set("dock_drives", PackedStringArray::from_iter(drives.iter().map(|s| GString::from(s.as_str()))));
        }
        dict.set("building_id", best_idx as i64);
        // The outline the inspector rings, at the bulldoze highlight's height.
        if let Some(site) = core.allocator.building_sites.get(best_idx) {
            let y =
                site.support_height_m + crate::nodes::sim::editing::BULLDOZE_HIGHLIGHT_Y_OFFSET_M;
            let footprint = PackedVector3Array::from_iter(
                site.footprint_world
                    .iter()
                    .map(|p| Vector3::new(p.x, y, p.y)),
            );
            dict.set("footprint", footprint);
        }
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
            i32::from(b.construction_hours_left(core.time.absolute_slot())),
        );
        dict.set(
            "construction_progress",
            f64::from(b.construction_progress(core.construction_clock_slots())),
        );
        dict.set("occupancy", b.occupancy as i32);
        let usage = b.recent_electricity;
        let consumed_kwh = usage.workplace.consumed_kwh + usage.residential.consumed_kwh;
        let local_kwh = usage.workplace.local_kwh + usage.residential.local_kwh;
        dict.set("electricity_consumption_kwh", consumed_kwh as f64);
        dict.set("electricity_local_kwh", local_kwh as f64);
        dict.set(
            "electricity_imported_kwh",
            (consumed_kwh - local_kwh).max(0.0) as f64,
        );
        if let (Ok(catalog), Ok(tuning)) = (
            crate::simulation::economy::definitions::load_runtime_economy_catalog(),
            crate::simulation::economy::definitions::load_runtime_economy_tuning(),
        ) {
            let price =
                crate::simulation::economy::households::local_utility_service_costs(&catalog)[0];
            let bill = usage
                .workplace
                .bill(price, tuning.owa_import_price_multiplier)
                .total()
                + usage
                    .residential
                    .bill(price, tuning.owa_import_price_multiplier)
                    .total();
            dict.set("electricity_cost", bill as f64);
        }
        dict.set(
            "household_capacity",
            core.allocator.household_capacity(best_idx),
        );
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
                dict.set("extractor_area_m2", f64::from(site.area_m2.max(0.0)));
                dict.set("extractor_total_reserve_units", total as f64);
                dict.set("extractor_available_reserve_units", available as f64);
                dict.set("extractor_consumed_reserve_units", consumed as f64);
                dict.set("extractor_consumed_reserve_ratio", consumed_ratio as f64);
            } else {
                dict.set("extractor_has_site", false);
                dict.set("extractor_area_m2", 0.0f64);
                dict.set("extractor_total_reserve_units", 0.0f64);
                dict.set("extractor_available_reserve_units", 0.0f64);
                dict.set("extractor_consumed_reserve_units", 0.0f64);
                dict.set("extractor_consumed_reserve_ratio", 0.0f64);
            }
        }
        if let Some(resource_id) = core.allocator.registry.field_resource(&b.asset_id) {
            dict.set("field_resource", GString::from(resource_id));
            Self::set_production_plot_boundaries(&core, best_idx, &mut dict);
            if let Some(site) = core.agriculture.site_for_building(best_idx) {
                dict.set("field_has_site", true);
                dict.set("field_area_m2", f64::from(site.area_m2.max(0.0)));
                dict.set(
                    "field_polygon",
                    PackedVector2Array::from(site.polygon_world.as_slice()),
                );
            } else {
                dict.set("field_has_site", false);
                dict.set("field_area_m2", 0.0f64);
            }
        }

        // Reuse the inspector's household aggregation for farms, including residents awaiting
        // rehousing from an inactive farm. This read runs on inspection/hour refresh, not each tick.
        let has_household_details = b.zone_type == ZoneType::Residential
            || core.allocator.registry.is_field_producer_asset(&b.asset_id);
        let mut total_agents = 0i32;
        let mut child_agents = 0i32;
        let mut adult_agents = 0i32;
        let mut elder_agents = 0i32;
        let mut household_count = 0i32;
        let mut household_budget_total = 0.0f32;
        let mut household_power_arrears = 0.0f32;
        let mut household_stock_total = 0.0f32;
        let mut household_stock_days_total = 0.0f32;
        let mut household_stock_days_min = f32::INFINITY;
        let mut household_replenishment_active = 0i32;
        let mut first_replenishment_state = None;
        let mut mixed_replenishment_state = false;
        if has_household_details {
            for h in &core.households.households {
                if h.home_building_id == best_idx {
                    household_count += 1;
                    total_agents += h.member_count as i32;
                    child_agents += h.child_count as i32;
                    adult_agents += h.adult_count as i32;
                    elder_agents += h.elder_count as i32;
                    household_budget_total += h.budget;
                    household_power_arrears += h.power_arrears.total();
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
        if has_household_details {
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
            dict.set("household_power_arrears", household_power_arrears as f64);
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
        if b.zone_type != ZoneType::Residential
            && let Some(profile) = profile
        {
            let factors = building_operation_factors(catalog, b, profile);
            let inventory_fill = building_inventory_fill_ratio(catalog, b, profile);
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
            } else if factors.active_worker_capacity < profile.worker_capacity
                && !profile_kind_uses_explicit_work_area(profile.kind)
            {
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
                if let Some(unit) = profile.utility_quantity_unit {
                    dict.set("utility_quantity_unit", GString::from(unit.symbol()));
                }
                dict.set("utility_service_available", utility_active);
                dict.set("utility_local_revenue", b.revenue as f64);
                if utility_service == "power" {
                    dict.set("power_capacity_kw", profile.power_capacity_kw as f64);
                    dict.set(
                        "power_staffed_capacity_kw",
                        (profile.power_capacity_kw
                            * b.worker_count.min(profile.worker_capacity) as f32
                            / profile.worker_capacity.max(1) as f32) as f64,
                    );
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
                    "city_input_cost_today",
                    b.daily_city_funded_input_cost as f64,
                );
            }
        }
        dict.set("budget_distress", b.budget_distress);
        dict.set("economy_broken", b.economy_broken);
        dict.set("broken", b.broken);
        dict.set("is_deserted", b.is_deserted);
        dict.set("pending_redevelopment", b.pending_redevelopment);
        dict.set("rezone_grace_days", b.rezone_grace_days_remaining as i32);
        dict.set("economy_profile", GString::from(profile_id));
        dict.set("inventory", inv_arr.to_variant());
        dict
    }

    /// Returns the simulation's default interior area for a newly authored farmhouse.
    #[func]
    pub fn get_default_farmhouse_area_m2(&self) -> f32 {
        crate::assets::asset::BuildingData::DEFAULT_FARMHOUSE_AREA_M2
    }

    /// The asset editor browser's rows for the registered assets matching `query` (`TOOLS-34`),
    /// grouped and ordered by Rust, as `{packs, missing}`. `packs` holds `{pack, asset_count,
    /// groups}`, each group `{label, type, asset_count, rows}` (`type` is the editor creation
    /// type a New asset there starts as, or `""`), each row `{label, asset_set, levels}` and
    /// each level `{level, id}` (`level` 0 for an asset that is not a building). Counts are of
    /// assets, not rows; a family's row holds only its matching members. `missing` holds the
    /// catalogue groups no pack has an asset in, as `{label, type}`, and is empty while a
    /// search is given. Call it on a refresh or a search edit, not per frame; see
    /// `assets::browser` for the order and the bound.
    #[func]
    pub fn get_asset_browser_rows(&self, query: GString) -> VarDictionary {
        use crate::assets::browser::{
            BrowserGroup, BrowserQuery, browser_packs, catalogue_groups, executable_services,
            missing_groups, profile_resources,
        };
        use crate::simulation::buildings::allocator::zone_type_to_zone_class;
        use crate::simulation::economy::definitions::EconomyProfileRuntimeKind as Kind;
        let core = self.lock_core();
        let registry = &core.allocator.registry;
        let query = BrowserQuery::parse(&query.to_string());
        let array = |items: Vec<VarDictionary>| {
            VarArray::from_iter(items.iter().map(|item| item.to_variant()))
        };
        let group_dict = |group: &BrowserGroup| {
            vdict! { "label": group.label(), "type": group.creation_type() }
        };
        let packs = browser_packs(registry, &query);
        let packs = packs.iter().map(|pack| {
            let groups = pack.groups.iter().map(|group| {
                let rows = group.rows.iter().map(|row| {
                    let levels = row.levels.iter().map(|level| {
                        vdict! { "level": i64::from(level.level), "id": level.id }
                    });
                    vdict! {
                        "label": row.label,
                        "asset_set": row.asset_set.unwrap_or_default(),
                        "levels": array(levels.collect()),
                    }
                });
                let mut dict = group_dict(&group.group);
                dict.set("asset_count", group.asset_count as i64);
                dict.set("rows", array(rows.collect()));
                dict
            });
            vdict! {
                "pack": pack.pack_id,
                "asset_count": pack.asset_count as i64,
                "groups": array(groups.collect()),
            }
        });
        let packs = array(packs.collect());
        let mut missing = Vec::new();
        if query.is_empty() {
            let catalog = core.demand.runtime_catalog();
            let zoned = core
                .zoning
                .profiles
                .profiles()
                .iter()
                .filter_map(|profile| {
                    let zone = zone_type_to_zone_class(profile.zone_type)?;
                    Some((zone, profile.density.as_str()))
                });
            let catalogue = catalogue_groups(
                zoned,
                executable_services(catalog),
                profile_resources(catalog, Kind::Extractor),
                profile_resources(catalog, Kind::FieldProducer),
            );
            missing = missing_groups(registry, catalogue)
                .iter()
                .map(group_dict)
                .collect();
        }
        vdict! { "packs": packs, "missing": array(missing) }
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
