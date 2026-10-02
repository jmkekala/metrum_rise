// SPDX-License-Identifier: GPL-2.0-only

//! Paper balance checks for the shipped `economy/profiles.toml` (`ECON-11`).
//!
//! These evaluate authored recipes, wages and prices at full staffing without running the
//! simulation. Outputs sell, and locally produced inputs are bought, at the catalog unit price;
//! import-only inputs pay the `OWA` import multiplier. Utility bills, property and profit tax,
//! freight and export saturation are left out, so passing is necessary for a solvent business but
//! not sufficient. See `docs/economy.md#growth-redesign-econ-11econ-14-dem-02`.

use super::runtime::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
    RuntimeEconomyTuning,
};
use super::{load_runtime_economy_catalog, load_runtime_economy_tuning};
use crate::simulation::work_area::{
    minimum_work_area_workers, profile_kind_uses_explicit_work_area,
};

/// Closed-chain jobs per 1,000 residents for the shipped profiles. Update it deliberately when
/// a tuning change moves the figure; `ECON-14` is expected to.
const CLOSED_CHAIN_JOBS_PER_1000_RESIDENTS: f64 = 130.55;

/// Privately run profiles whose payroll must be covered by sales.
fn is_business(kind: EconomyProfileRuntimeKind) -> bool {
    use EconomyProfileRuntimeKind::*;
    matches!(
        kind,
        Producer | FieldProducer | Processor | Store | ServiceStore | Extractor
    )
}

/// Industrial profiles that may export to the `OWA`. Producers and processors export only from
/// industrial zones, which is where the shipped assets place them.
fn is_basic_industry(kind: EconomyProfileRuntimeKind) -> bool {
    use EconomyProfileRuntimeKind::*;
    matches!(kind, Producer | FieldProducer | Processor | Extractor)
}

/// Workers per building, or per hectare for area-staffed profiles.
fn workers_per_unit_scale(profile: &EconomyProfileRuntime) -> f64 {
    if profile_kind_uses_explicit_work_area(profile.kind) {
        f64::from(profile.workers_per_hectare)
    } else {
        f64::from(profile.worker_capacity)
    }
}

/// One building, or for area-staffed profiles the smallest area (in hectares, at least one) at
/// which the field staffing floor no longer binds.
fn reference_scale(profile: &EconomyProfileRuntime) -> f64 {
    let floor_workers = f64::from(minimum_work_area_workers(profile.kind));
    let workers = workers_per_unit_scale(profile);
    if profile_kind_uses_explicit_work_area(profile.kind) && workers > 0.0 {
        (floor_workers / workers).max(1.0)
    } else {
        1.0
    }
}

/// Inputs bought from other businesses; same-resource upkeep is netted out of the output.
fn purchased_inputs(
    profile: &EconomyProfileRuntime,
) -> impl Iterator<Item = (ResourceRuntimeId, f64)> {
    profile
        .inputs
        .iter()
        .filter(|port| profile.output_port(port.resource_runtime_id).is_none())
        .map(|port| (port.resource_runtime_id, f64::from(port.units_per_day)))
}

fn resource_name(catalog: &RuntimeEconomyCatalog, resource: ResourceRuntimeId) -> &str {
    catalog
        .resource_id_for_runtime_id(resource)
        .unwrap_or("<unknown>")
}

/// The single business profile producing `resource`, or `None` when it is import-only.
fn local_producer(
    catalog: &RuntimeEconomyCatalog,
    resource: ResourceRuntimeId,
) -> Option<&EconomyProfileRuntime> {
    let mut producers = catalog
        .all_profiles()
        .iter()
        .filter(|profile| is_business(profile.kind) && profile.output_port(resource).is_some());
    let producer = producers.next()?;
    assert!(
        producers.next().is_none(),
        "resource '{}' has several producing profiles; the chain model needs one",
        resource_name(catalog, resource)
    );
    assert_eq!(
        producer.outputs.len(),
        1,
        "profile '{}' has several outputs; the chain model needs one",
        producer.id
    );
    Some(producer)
}

fn unit_price(catalog: &RuntimeEconomyCatalog, resource: ResourceRuntimeId) -> f64 {
    f64::from(
        catalog
            .unit_price_for_resource(resource)
            .expect("compiled resources have a price"),
    )
}

fn purchase_price(
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    resource: ResourceRuntimeId,
) -> f64 {
    let price = unit_price(catalog, resource);
    if local_producer(catalog, resource).is_some() {
        price
    } else {
        price * f64::from(tuning.owa_import_price_multiplier)
    }
}

/// Daily money flows of one profile at its reference scale and full staffing.
struct DailyBalance {
    revenue: f64,
    wages: f64,
    input_cost: f64,
}

impl DailyBalance {
    /// `sale_multiplier` scales the catalog price of every output (`1.0` for local sales).
    fn at_full_staffing(
        profile: &EconomyProfileRuntime,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        sale_multiplier: f64,
    ) -> Self {
        let scale = reference_scale(profile);
        let revenue: f64 = profile
            .outputs
            .iter()
            .map(|port| {
                f64::from(profile.net_output_units_per_day(port))
                    * unit_price(catalog, port.resource_runtime_id)
                    * sale_multiplier
            })
            .sum();
        let input_cost: f64 = purchased_inputs(profile)
            .map(|(resource, units)| units * purchase_price(catalog, tuning, resource))
            .sum();
        Self {
            revenue: revenue * scale,
            wages: workers_per_unit_scale(profile)
                * scale
                * f64::from(profile.wage_max_currency_per_day),
            input_cost: input_cost * scale,
        }
    }

    fn profit(&self) -> f64 {
        self.revenue - self.wages - self.input_cost
    }
}

/// Lists every profile selected by `include` that loses money at `sale_multiplier`.
fn loss_making_profiles(
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    include: fn(EconomyProfileRuntimeKind) -> bool,
    sale_multiplier: f64,
) -> Vec<String> {
    catalog
        .all_profiles()
        .iter()
        .filter(|profile| include(profile.kind))
        .filter_map(|profile| {
            let balance = DailyBalance::at_full_staffing(profile, catalog, tuning, sale_multiplier);
            (balance.profit() < 0.0).then(|| {
                format!(
                    "{}: revenue {:.1} - wages {:.1} - inputs {:.1} = {:.1}/day",
                    profile.id,
                    balance.revenue,
                    balance.wages,
                    balance.input_cost,
                    balance.profit()
                )
            })
        })
        .collect()
}

/// Jobs needed to supply `units_per_day` of `resource` through local producers, upstream
/// included. Area-staffed producers are counted per hectare without the per-field staffing
/// floor, which stops binding at city scale.
fn jobs_to_supply(
    catalog: &RuntimeEconomyCatalog,
    resource: ResourceRuntimeId,
    units_per_day: f64,
    depth: usize,
) -> f64 {
    assert!(
        depth <= catalog.all_profiles().len(),
        "supply chain for '{}' is cyclic",
        resource_name(catalog, resource)
    );
    let Some(producer) = local_producer(catalog, resource) else {
        return 0.0;
    };
    let port = producer
        .output_port(resource)
        .expect("producer outputs resource");
    let net_output = f64::from(producer.net_output_units_per_day(port));
    assert!(
        net_output > 0.0,
        "profile '{}' produces no net '{}'",
        producer.id,
        resource_name(catalog, resource)
    );
    let scale = units_per_day / net_output;
    let upstream: f64 = purchased_inputs(producer)
        .map(|(input, units)| jobs_to_supply(catalog, input, units * scale, depth + 1))
        .sum();
    workers_per_unit_scale(producer) * scale + upstream
}

/// Jobs per resident in the closed supply chain: everything residents consume, made locally,
/// with no exports. City-owned utilities are excluded.
fn closed_chain_jobs_per_resident(catalog: &RuntimeEconomyCatalog) -> f64 {
    catalog
        .all_profiles()
        .iter()
        .filter(|profile| profile.kind == EconomyProfileRuntimeKind::DemandSink)
        .flat_map(|sink| {
            sink.inputs
                .iter()
                .map(|port| (port.resource_runtime_id, sink.consumption_rate_per_resident))
        })
        .map(|(resource, rate)| jobs_to_supply(catalog, resource, f64::from(rate), 0))
        .sum()
}

#[test]
fn shipped_businesses_are_solvent_at_full_staffing() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let losses = loss_making_profiles(&catalog, &tuning, is_business, 1.0);
    assert!(
        losses.is_empty(),
        "insolvent profiles:\n{}",
        losses.join("\n")
    );
}

#[test]
fn solvency_check_reports_a_profile_whose_wages_exceed_revenue() {
    let mut catalog = (*load_runtime_economy_catalog().unwrap()).clone();
    let tuning = load_runtime_economy_tuning().unwrap();
    let grocery = catalog
        .profiles
        .iter_mut()
        .find(|profile| profile.id == "grocery_basic")
        .unwrap();
    grocery.wage_max_currency_per_day = 1_000.0;
    let losses = loss_making_profiles(&catalog, &tuning, is_business, 1.0);
    assert_eq!(losses.len(), 1, "{losses:?}");
    assert!(losses[0].starts_with("grocery_basic:"), "{}", losses[0]);
}

#[test]
#[ignore = "ECON-14: exports pay owa_export_price_multiplier = 0.60, so basic industries lose money \
            selling to the OWA"]
fn basic_industries_profit_from_exporting_all_output() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let losses = loss_making_profiles(
        &catalog,
        &tuning,
        is_basic_industry,
        f64::from(tuning.owa_export_price_multiplier),
    );
    assert!(
        losses.is_empty(),
        "basic industries losing money on exports:\n{}",
        losses.join("\n")
    );
}

#[test]
fn closed_chain_jobs_per_thousand_residents_matches_recorded_baseline() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let jobs = closed_chain_jobs_per_resident(&catalog) * 1_000.0;
    assert!(
        (jobs - CLOSED_CHAIN_JOBS_PER_1000_RESIDENTS).abs() < 0.01,
        "closed chain gives {jobs:.2} jobs per 1,000 residents; recorded baseline is \
         {CLOSED_CHAIN_JOBS_PER_1000_RESIDENTS}"
    );
}
