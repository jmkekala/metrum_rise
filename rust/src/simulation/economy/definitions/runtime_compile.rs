//! Compilation from authored economy profiles into compact runtime catalog data.

use super::runtime::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
    RuntimeEconomyTuning, RuntimeResourcePort,
};
use super::schema::{AuthoredProfileKind, EconomyProfile, PROFILE_KIND_DEMAND_SINK};
use std::collections::{BTreeMap, BTreeSet};

const REQUIRED_HOUSEHOLD_DEMAND_PROFILE_ID: &str = "basic_household_demand";
const REQUIRED_HOUSEHOLD_SUPPLY_RESOURCE_ID: &str = "household_supplies";
const LEGACY_RESOURCE_ORDER: [&str; 3] = ["household_supplies", "packaged_food", "grain"];

pub(super) fn compile_runtime_catalog(
    authored_profiles: &[EconomyProfile],
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
    let mut resource_ids = BTreeSet::new();
    for profile in authored_profiles {
        for input in &profile.inputs {
            resource_ids.insert(input.resource.clone());
        }
        for output in &profile.outputs {
            resource_ids.insert(output.resource.clone());
        }
    }
    let mut ordered_resource_ids = Vec::with_capacity(resource_ids.len());
    for resource_id in LEGACY_RESOURCE_ORDER {
        if resource_ids.remove(resource_id) {
            ordered_resource_ids.push(resource_id.to_owned());
        }
    }
    ordered_resource_ids.extend(resource_ids);
    for (idx, resource_id) in ordered_resource_ids.into_iter().enumerate() {
        let runtime_id = u16::try_from(idx + 1)
            .map_err(|_| "runtime economy catalog exceeds u16 resource id range".to_owned())?;
        catalog
            .resource_by_id
            .insert(resource_id.clone(), runtime_id);
        catalog.resource_id_by_runtime_id.push(resource_id);
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
            let Some(unit_price) = catalog.unit_price_for_resource(port.resource_runtime_id) else {
                let resource_id = catalog
                    .resource_id_for_runtime_id(port.resource_runtime_id)
                    .unwrap_or("<unknown>");
                return Err(format!(
                    "resource '{resource_id}' used by profile '{}' must have a positive unit price",
                    profile.id
                ));
            };
            if unit_price <= 0.0 {
                let resource_id = catalog
                    .resource_id_for_runtime_id(port.resource_runtime_id)
                    .unwrap_or("<unknown>");
                return Err(format!(
                    "resource '{resource_id}' used by profile '{}' must have a positive unit price",
                    profile.id
                ));
            }
        }
    }

    let household_profile = catalog
        .profile_for_id(REQUIRED_HOUSEHOLD_DEMAND_PROFILE_ID)
        .ok_or_else(|| {
            format!(
                "runtime economy catalog requires profile '{REQUIRED_HOUSEHOLD_DEMAND_PROFILE_ID}'"
            )
        })?;
    if household_profile.kind != EconomyProfileRuntimeKind::DemandSink {
        return Err(format!(
            "profile '{REQUIRED_HOUSEHOLD_DEMAND_PROFILE_ID}' must have kind = \"{PROFILE_KIND_DEMAND_SINK}\""
        ));
    }
    if household_profile.consumption_rate_per_resident <= 0.0 {
        return Err(format!(
            "profile '{REQUIRED_HOUSEHOLD_DEMAND_PROFILE_ID}'.consumption_rate_per_resident must be > 0"
        ));
    }
    if household_profile.stock_target_days <= 0.0 {
        return Err(format!(
            "profile '{REQUIRED_HOUSEHOLD_DEMAND_PROFILE_ID}'.stock_target_days must be > 0"
        ));
    }
    if household_profile.reorder_threshold_days <= 0.0 {
        return Err(format!(
            "profile '{REQUIRED_HOUSEHOLD_DEMAND_PROFILE_ID}'.reorder_threshold_days must be > 0"
        ));
    }
    let household_supplies = catalog
        .resource_runtime_id_for_id(REQUIRED_HOUSEHOLD_SUPPLY_RESOURCE_ID)
        .ok_or_else(|| {
            format!("runtime economy catalog requires resource '{REQUIRED_HOUSEHOLD_SUPPLY_RESOURCE_ID}'")
        })?;
    if !household_profile
        .inputs
        .iter()
        .any(|port| port.resource_runtime_id == household_supplies)
    {
        return Err(format!(
            "profile '{REQUIRED_HOUSEHOLD_DEMAND_PROFILE_ID}' must consume resource '{REQUIRED_HOUSEHOLD_SUPPLY_RESOURCE_ID}'"
        ));
    }
    let Some(unit_price) = catalog.unit_price_for_resource(household_supplies) else {
        return Err(format!(
            "resource '{REQUIRED_HOUSEHOLD_SUPPLY_RESOURCE_ID}' must have a positive unit price"
        ));
    };
    if unit_price <= 0.0 {
        return Err(format!(
            "resource '{REQUIRED_HOUSEHOLD_SUPPLY_RESOURCE_ID}' must have a positive unit price"
        ));
    }
    Ok(())
}

fn compile_runtime_profile(
    runtime_id: u16,
    profile: &EconomyProfile,
    resource_by_id: &BTreeMap<String, ResourceRuntimeId>,
) -> Result<EconomyProfileRuntime, String> {
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

    let compiled_inputs = profile
        .inputs
        .iter()
        .map(|input| {
            let Some(&resource_runtime_id) = resource_by_id.get(input.resource.as_str()) else {
                return Err(format!(
                    "profile '{}' references unresolved input resource '{}'",
                    profile.id, input.resource
                ));
            };
            Ok(RuntimeResourcePort {
                resource_runtime_id,
                units_per_day: input.units_per_day.max(0.0),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let compiled_outputs = profile
        .outputs
        .iter()
        .map(|output| {
            let Some(&resource_runtime_id) = resource_by_id.get(output.resource.as_str()) else {
                return Err(format!(
                    "profile '{}' references unresolved output resource '{}'",
                    profile.id, output.resource
                ));
            };
            Ok(RuntimeResourcePort {
                resource_runtime_id,
                units_per_day: output.units_per_day.max(0.0),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

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
        unit_price_currency: profile.unit_price_currency.max(0.0),
        base_rate_units_per_day: profile.base_rate_units_per_day.max(0.0),
        wage_min_currency_per_day: profile.wage_min_currency_per_day.max(0.0),
        wage_max_currency_per_day: profile.wage_max_currency_per_day.max(0.0),
        worker_capacity: profile.worker_capacity,
        stock_target_days: profile.stock_target_days.max(0.0),
        starting_inventory_days: profile.starting_inventory_days.max(0.0),
        reorder_threshold_days: profile.reorder_threshold_days.max(0.0),
        critical_threshold_days: profile.critical_threshold_days.max(0.0),
        min_shipment_units: profile.min_shipment_units.max(0.0),
        consumption_rate_per_resident: profile.consumption_rate_per_resident.max(0.0),
        utility_service: profile.utility_service.clone(),
        inputs: compiled_inputs,
        outputs: compiled_outputs,
        runtime_supported,
    })
}
