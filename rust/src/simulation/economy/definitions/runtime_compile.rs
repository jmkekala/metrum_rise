// SPDX-License-Identifier: GPL-2.0-only

//! Compilation from authored economy profiles into compact runtime catalog data.

use super::runtime::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
    RuntimeEconomyTuning, RuntimeResourcePort,
};
use super::schema::{
    AuthoredProfileKind, EconomyProfile, EconomyResource, PROFILE_KIND_DEMAND_SINK, ResourcePort,
};
use super::validation::validate_range;
use std::collections::{BTreeMap, BTreeSet};

use super::{HOUSEHOLD_DEMAND_PROFILE_ID, HOUSEHOLD_SUPPLY_RESOURCE_ID};

pub(super) fn compile_runtime_catalog(
    authored_profiles: &[EconomyProfile],
    authored_resources: &[EconomyResource],
    runtime_tuning: &RuntimeEconomyTuning,
) -> Result<RuntimeEconomyCatalog, String> {
    let work_profile_ids: BTreeSet<&str> = runtime_tuning
        .operational_clock
        .work_profiles
        .iter()
        .map(|profile| profile.id.as_str())
        .collect();
    let freight_profile_ids: BTreeSet<&str> = runtime_tuning
        .operational_clock
        .freight_profiles
        .iter()
        .map(|profile| profile.id.as_str())
        .collect();

    let mut catalog = RuntimeEconomyCatalog::default();
    let mut resources: Vec<_> = authored_resources.iter().collect();
    resources.sort_unstable_by_key(|resource| resource.runtime_id);
    for (idx, resource) in resources.into_iter().enumerate() {
        if usize::from(resource.runtime_id) != idx + 1 {
            return Err(
                "resource runtime IDs must be unique and contiguous starting at 1".to_owned(),
            );
        }
        if resource.id.trim().is_empty()
            || catalog
                .resource_by_id
                .insert(resource.id.clone(), resource.runtime_id)
                .is_some()
        {
            return Err(format!(
                "resource name '{}' must be nonempty and unique",
                resource.id
            ));
        }
        catalog.resource_id_by_runtime_id.push(resource.id.clone());
        if let Some(price) = resource.import_unit_price_currency {
            if !price.is_finite() || price <= 0.0 {
                return Err(format!(
                    "resource '{}' import price must be finite and positive",
                    resource.id
                ));
            }
            catalog.price_by_resource.insert(resource.runtime_id, price);
        }
    }

    for (idx, profile) in authored_profiles.iter().enumerate() {
        if catalog.by_id.contains_key(&profile.id) {
            return Err(format!(
                "runtime economy catalog contains duplicate profile id '{}'",
                profile.id
            ));
        }
        if let Some(work_profile) = profile.work_schedule_profile.as_deref()
            && !work_profile_ids.contains(work_profile)
        {
            return Err(format!(
                "profile '{}' references missing work_schedule_profile '{}'",
                profile.id, work_profile
            ));
        }
        if let Some(freight_profile) = profile.freight_timing_profile.as_deref()
            && !freight_profile_ids.contains(freight_profile)
        {
            return Err(format!(
                "profile '{}' references missing freight_timing_profile '{}'",
                profile.id, freight_profile
            ));
        }

        let runtime_id = u16::try_from(idx + 1)
            .map_err(|_| "runtime economy catalog exceeds u16 profile id range".to_owned())?;
        let compiled = compile_runtime_profile(runtime_id, profile, &catalog.resource_by_id)?;
        if compiled.unit_price_currency > 0.0 {
            for output in &compiled.outputs {
                catalog
                    .price_by_resource
                    .entry(output.resource_runtime_id)
                    .or_insert(compiled.unit_price_currency);
            }
        }
        catalog.by_id.insert(compiled.id.clone(), runtime_id);
        catalog.profiles.push(compiled);
    }

    validate_required_runtime_catalog(&catalog)?;
    Ok(catalog)
}

fn validate_required_runtime_catalog(catalog: &RuntimeEconomyCatalog) -> Result<(), String> {
    for profile in catalog.all_profiles() {
        for port in profile.inputs.iter().chain(profile.outputs.iter()) {
            if !catalog
                .unit_price_for_resource(port.resource_runtime_id)
                .is_some_and(|price| price.is_finite() && price > 0.0)
            {
                let resource_id = catalog
                    .resource_id_for_runtime_id(port.resource_runtime_id)
                    .unwrap_or("<unknown>");
                return Err(format!(
                    "resource '{resource_id}' used by profile '{}' must have a finite positive unit price",
                    profile.id
                ));
            }
        }
    }

    let household_profile = catalog
        .profile_for_id(HOUSEHOLD_DEMAND_PROFILE_ID)
        .ok_or_else(|| {
            format!("runtime economy catalog requires profile '{HOUSEHOLD_DEMAND_PROFILE_ID}'")
        })?;
    if household_profile.kind != EconomyProfileRuntimeKind::DemandSink {
        return Err(format!(
            "profile '{HOUSEHOLD_DEMAND_PROFILE_ID}' must have kind = \"{PROFILE_KIND_DEMAND_SINK}\""
        ));
    }
    if household_profile.consumption_rate_per_resident <= 0.0 {
        return Err(format!(
            "profile '{HOUSEHOLD_DEMAND_PROFILE_ID}'.consumption_rate_per_resident must be > 0"
        ));
    }
    if household_profile.stock_target_days <= 0.0 {
        return Err(format!(
            "profile '{HOUSEHOLD_DEMAND_PROFILE_ID}'.stock_target_days must be > 0"
        ));
    }
    if household_profile.reorder_threshold_days <= 0.0 {
        return Err(format!(
            "profile '{HOUSEHOLD_DEMAND_PROFILE_ID}'.reorder_threshold_days must be > 0"
        ));
    }
    let household_supplies = catalog
        .resource_runtime_id_for_id(HOUSEHOLD_SUPPLY_RESOURCE_ID)
        .ok_or_else(|| {
            format!("runtime economy catalog requires resource '{HOUSEHOLD_SUPPLY_RESOURCE_ID}'")
        })?;
    if !household_profile
        .inputs
        .iter()
        .any(|port| port.resource_runtime_id == household_supplies)
    {
        return Err(format!(
            "profile '{HOUSEHOLD_DEMAND_PROFILE_ID}' must consume resource '{HOUSEHOLD_SUPPLY_RESOURCE_ID}'"
        ));
    }
    Ok(())
}

fn compile_runtime_profile(
    runtime_id: u16,
    profile: &EconomyProfile,
    resource_by_id: &BTreeMap<String, ResourceRuntimeId>,
) -> Result<EconomyProfileRuntime, String> {
    for (field, value) in [
        ("base_rate_units_per_day", profile.base_rate_units_per_day),
        (
            "wage_min_currency_per_day",
            profile.wage_min_currency_per_day,
        ),
        (
            "wage_max_currency_per_day",
            profile.wage_max_currency_per_day,
        ),
        ("unit_price_currency", profile.unit_price_currency),
        ("stock_target_days", profile.stock_target_days),
        ("starting_inventory_days", profile.starting_inventory_days),
        ("reorder_threshold_days", profile.reorder_threshold_days),
        ("critical_threshold_days", profile.critical_threshold_days),
        ("min_shipment_units", profile.min_shipment_units),
        (
            "consumption_rate_per_resident",
            profile.consumption_rate_per_resident,
        ),
    ] {
        validate_range(
            value,
            0.0,
            f32::MAX,
            &format!("profile '{}'.{field}", profile.id),
        )?;
    }
    if !profile.worker_capacity_area_m2.is_finite() || profile.worker_capacity_area_m2 <= 0.0 {
        return Err(format!(
            "profile '{}'.worker_capacity_area_m2 must be finite and > 0",
            profile.id
        ));
    }
    let workers_per_hectare = profile.workers_per_hectare();
    if !workers_per_hectare.is_finite() {
        return Err(format!(
            "profile '{}'.worker_capacity_area_m2 produces a non-finite worker density",
            profile.id
        ));
    }
    let kind = match profile.authored_kind() {
        AuthoredProfileKind::Producer => EconomyProfileRuntimeKind::Producer,
        AuthoredProfileKind::FieldProducer => EconomyProfileRuntimeKind::FieldProducer,
        AuthoredProfileKind::Processor => EconomyProfileRuntimeKind::Processor,
        AuthoredProfileKind::Store => EconomyProfileRuntimeKind::Store,
        AuthoredProfileKind::ServiceStore => EconomyProfileRuntimeKind::ServiceStore,
        AuthoredProfileKind::DemandSink => EconomyProfileRuntimeKind::DemandSink,
        AuthoredProfileKind::Extractor => EconomyProfileRuntimeKind::Extractor,
        AuthoredProfileKind::UtilityProducer => EconomyProfileRuntimeKind::UtilityProducer,
        AuthoredProfileKind::UtilityProcessor => EconomyProfileRuntimeKind::UtilityProcessor,
        AuthoredProfileKind::Unsupported => EconomyProfileRuntimeKind::Unsupported,
    };

    let compiled_inputs =
        compile_resource_ports(profile, &profile.inputs, "input", resource_by_id)?;
    let compiled_outputs =
        compile_resource_ports(profile, &profile.outputs, "output", resource_by_id)?;

    let runtime_supported = match kind {
        EconomyProfileRuntimeKind::Producer
        | EconomyProfileRuntimeKind::FieldProducer
        | EconomyProfileRuntimeKind::Extractor => !compiled_outputs.is_empty(),
        EconomyProfileRuntimeKind::Processor | EconomyProfileRuntimeKind::Store => {
            !compiled_inputs.is_empty() && !compiled_outputs.is_empty()
        }
        EconomyProfileRuntimeKind::ServiceStore => !compiled_outputs.is_empty(),
        EconomyProfileRuntimeKind::UtilityProducer
        | EconomyProfileRuntimeKind::UtilityProcessor => true,
        EconomyProfileRuntimeKind::DemandSink | EconomyProfileRuntimeKind::Unsupported => false,
    };

    Ok(EconomyProfileRuntime {
        runtime_id,
        id: profile.id.clone(),
        kind,
        work_schedule_profile: profile.work_schedule_profile.clone(),
        freight_timing_profile: profile.freight_timing_profile.clone(),
        unit_price_currency: profile.unit_price_currency,
        base_rate_units_per_day: profile.base_rate_units_per_day,
        wage_min_currency_per_day: profile.wage_min_currency_per_day,
        wage_max_currency_per_day: profile.wage_max_currency_per_day,
        worker_capacity: profile.worker_capacity,
        workers_per_hectare,
        stock_target_days: profile.stock_target_days,
        starting_inventory_days: profile.starting_inventory_days,
        reorder_threshold_days: profile.reorder_threshold_days,
        critical_threshold_days: profile.critical_threshold_days,
        min_shipment_units: profile.min_shipment_units,
        consumption_rate_per_resident: profile.consumption_rate_per_resident,
        utility_service: profile.utility_service.clone(),
        inputs: compiled_inputs,
        outputs: compiled_outputs,
        runtime_supported,
    })
}

fn compile_resource_ports(
    profile: &EconomyProfile,
    ports: &[ResourcePort],
    direction: &str,
    resource_by_id: &BTreeMap<String, ResourceRuntimeId>,
) -> Result<Vec<RuntimeResourcePort>, String> {
    let mut seen = BTreeSet::new();
    ports
        .iter()
        .map(|port| {
            let Some(&resource_runtime_id) = resource_by_id.get(&port.resource) else {
                return Err(format!(
                    "profile '{}' references unresolved {direction} resource '{}'",
                    profile.id, port.resource
                ));
            };
            if !seen.insert(resource_runtime_id) {
                return Err(format!(
                    "profile '{}' repeats {direction} resource '{}'",
                    profile.id, port.resource
                ));
            }
            if !port.units_per_day.is_finite() || port.units_per_day < 0.0 {
                return Err(format!(
                    "profile '{}' {direction} resource '{}' rate must be finite and nonnegative",
                    profile.id, port.resource
                ));
            }
            Ok(RuntimeResourcePort {
                resource_runtime_id,
                units_per_day: port.units_per_day,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_profile_scalars_reject_nonfinite_and_negative_values() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../economy");
        let project = super::super::io::load_project(&path).unwrap();
        let resources: BTreeMap<_, _> = project
            .resources
            .iter()
            .map(|resource| (resource.id.clone(), resource.runtime_id))
            .collect();
        let profile = project
            .profiles
            .iter()
            .find(|profile| profile.id == "machinery_factory_basic")
            .unwrap();
        let scalar_fields: [fn(&mut EconomyProfile) -> &mut f32; 10] = [
            |p| &mut p.base_rate_units_per_day,
            |p| &mut p.wage_min_currency_per_day,
            |p| &mut p.wage_max_currency_per_day,
            |p| &mut p.unit_price_currency,
            |p| &mut p.stock_target_days,
            |p| &mut p.starting_inventory_days,
            |p| &mut p.reorder_threshold_days,
            |p| &mut p.critical_threshold_days,
            |p| &mut p.min_shipment_units,
            |p| &mut p.consumption_rate_per_resident,
        ];
        for (field, select) in scalar_fields.into_iter().enumerate() {
            for invalid in [-1.0, f32::NAN, f32::INFINITY] {
                let mut bad = profile.clone();
                *select(&mut bad) = invalid;
                assert!(
                    compile_runtime_profile(1, &bad, &resources).is_err(),
                    "accepted field {field} = {invalid}"
                );
            }
        }
    }

    #[test]
    fn recipe_ports_reject_duplicates_and_invalid_rates() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../economy");
        let project = super::super::io::load_project(&path).unwrap();
        let profile = project
            .profiles
            .iter()
            .find(|profile| profile.id == "machinery_factory_basic")
            .unwrap();
        let resources: BTreeMap<_, _> = project
            .resources
            .iter()
            .map(|resource| (resource.id.clone(), resource.runtime_id))
            .collect();
        for direction in ["input", "output"] {
            let mut bad = profile.clone();
            let ports = if direction == "input" {
                &mut bad.inputs
            } else {
                &mut bad.outputs
            };
            ports.push(ports[0].clone());
            assert!(
                compile_runtime_profile(1, &bad, &resources)
                    .unwrap_err()
                    .contains("repeats")
            );
            for invalid in [-1.0, f32::NAN, f32::INFINITY] {
                let mut bad = profile.clone();
                let ports = if direction == "input" {
                    &mut bad.inputs
                } else {
                    &mut bad.outputs
                };
                ports[0].units_per_day = invalid;
                assert!(
                    compile_runtime_profile(1, &bad, &resources)
                        .unwrap_err()
                        .contains("finite and nonnegative")
                );
            }
        }
    }

    #[test]
    fn staffing_reference_area_compiles_fractional_density_without_changing_output() {
        let mut profile: EconomyProfile = toml::from_str(
            "id = 'test_farm'\ndisplay_name = 'Farm'\nkind = 'field_producer'\nworker_capacity = 5\nbase_rate_units_per_day = 290.0",
        ).unwrap();
        let resources = BTreeMap::new();
        let default_area = compile_runtime_profile(1, &profile, &resources).unwrap();
        assert_eq!(default_area.workers_per_hectare, 5.0);
        profile.worker_capacity = 1;
        profile.worker_capacity_area_m2 = 100_000.0;
        let farm = compile_runtime_profile(1, &profile, &resources).unwrap();
        assert_eq!(farm.worker_capacity, 1);
        assert_eq!(farm.workers_per_hectare, 0.1);
        assert_eq!(farm.base_rate_units_per_day, 290.0);

        for invalid_area in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::MIN_POSITIVE] {
            profile.worker_capacity_area_m2 = invalid_area;
            assert!(compile_runtime_profile(1, &profile, &resources).is_err());
        }
    }
    #[test]
    fn shipped_resource_ids_survive_reordering_and_import_only_prices_roundtrip() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../economy");
        let mut project = super::super::io::load_project(&path).unwrap();
        project.resources.reverse();
        let catalog = compile_runtime_catalog(
            &project.profiles,
            &project.resources,
            &project.runtime_tuning,
        )
        .unwrap();
        for (id, name) in [
            (1, "household_supplies"),
            (2, "packaged_food"),
            (3, "grain"),
            (4, "coal"),
            (5, "health_essentials"),
            (6, "personal_services"),
            (7, "machinery"),
            (8, "steel"),
            (9, "metals"),
        ] {
            assert_eq!(catalog.resource_runtime_id_for_id(name), Some(id));
        }
        assert_eq!(catalog.unit_price_for_resource(8), Some(8.0));
        assert_eq!(catalog.unit_price_for_resource(9), Some(16.0));
        let encoded = serde_json::to_string(&project).unwrap();
        let project: super::super::schema::EconomyProject = serde_json::from_str(&encoded).unwrap();
        assert!(
            compile_runtime_catalog(
                &project.profiles,
                &project.resources,
                &project.runtime_tuning
            )
            .is_ok()
        );
        let mut duplicate = project.resources.clone();
        duplicate[0].runtime_id = duplicate[1].runtime_id;
        assert!(
            compile_runtime_catalog(&project.profiles, &duplicate, &project.runtime_tuning)
                .is_err()
        );
        let mut invalid_price = project.resources.clone();
        invalid_price[0].import_unit_price_currency = Some(f32::NAN);
        assert!(
            compile_runtime_catalog(&project.profiles, &invalid_price, &project.runtime_tuning)
                .is_err()
        );
    }
}
