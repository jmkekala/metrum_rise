// SPDX-License-Identifier: GPL-3.0-or-later

//! The store mix of a populated city with a second store size (`ECON-32`): groceries and kiosks
//! standing and built each day, and each kind's share of household-supply sales. Run once with
//! `METRUM_TEST_KIOSK=1` (a synthetic one-cell kiosk joins the building pack) and once without
//! (the shipped packs, as `dev` builds them):
//!
//! `METRUM_TEST_KIOSK=1 METRUM_KIOSK_MIX_RESIDENTS=10000 METRUM_KIOSK_MIX_DAYS=4 $TEST_BINARY
//! nodes::sim::core::populated_city::kiosk_mix::kiosk_mix --exact --ignored --nocapture`

use super::PopulatedCity;
use super::super::state::SimCore;
use crate::simulation::economy::definitions::EconomyProfileRuntimeKind;

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name).map_or(default, |value| {
        value
            .parse()
            .unwrap_or_else(|_| panic!("{name} must be an integer"))
    })
}

// One store profile's stores at a day's end.
#[derive(Debug)]
struct Row {
    profile: String,
    standing: u32,
    constructing: u32,
    // The standing stores' last completed day's household sales, in currency.
    sales: f64,
    // Their workers now, fewest and most in one store.
    workers: u32,
    crew: (u32, u32),
    // Their input stock (packaged food) now, kg: deliveries arrived.
    input_stock: f64,
}

fn census(core: &SimCore) -> Vec<Row> {
    let catalog = core.demand.runtime_catalog();
    let mut rows: Vec<Row> = Vec::new();
    for building in &core.allocator.buildings {
        let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
        else {
            continue;
        };
        if profile.kind != EconomyProfileRuntimeKind::Store
            || building.broken
            || building.economy_broken
            || building.is_deserted
        {
            continue;
        }
        let index = match rows.iter().position(|row| row.profile == profile.id) {
            Some(index) => index,
            None => {
                rows.push(Row {
                    profile: profile.id.clone(),
                    standing: 0,
                    constructing: 0,
                    sales: 0.0,
                    workers: 0,
                    crew: (u32::MAX, 0),
                    input_stock: 0.0,
                });
                rows.len() - 1
            }
        };
        let row = &mut rows[index];
        if building.is_under_construction() {
            row.constructing += 1;
        } else {
            row.standing += 1;
            row.sales += f64::from(building.recent_household_sales_value);
            row.workers += building.worker_count;
            row.crew = (
                row.crew.0.min(building.worker_count),
                row.crew.1.max(building.worker_count),
            );
            row.input_stock += profile.inputs.first().map_or(0.0, |port| {
                f64::from(building.inventory_units(port.resource_runtime_id))
            });
        }
    }
    rows.sort_by(|a, b| a.profile.cmp(&b.profile));
    rows
}

#[test]
#[ignore = "builds a populated city; run in a release test build"]
fn kiosk_mix() {
    let residents = env_u32("METRUM_KIOSK_MIX_RESIDENTS", 10_000);
    let days = env_u32("METRUM_KIOSK_MIX_DAYS", 4);
    let mut city = PopulatedCity::build(residents);
    let kiosk = std::env::var_os("METRUM_TEST_KIOSK").is_some();
    println!("kiosk_mix residents={residents} kiosk_asset={kiosk}");
    let mut previous = census(&city.core);
    println!("day {} after build: {previous:?}", city.core.time.day_index);
    for _ in 0..days {
        for _ in 0..24 {
            city.advance_hour();
        }
        let now = census(&city.core);
        let sales: f64 = now.iter().map(|row| row.sales).sum();
        for row in &now {
            let before = previous
                .iter()
                .find(|old| old.profile == row.profile)
                .map_or(0, |old| old.standing + old.constructing);
            println!(
                "day {} {}: standing {} constructing {} (net {:+} on the day before), sales {:.0} \
                 ({:.1}% of household supplies), workers {} (a store {}-{}), input stock {:.0} kg",
                city.core.time.day_index,
                row.profile,
                row.standing,
                row.constructing,
                i64::from(row.standing + row.constructing) - i64::from(before),
                row.sales,
                if sales > 0.0 { 100.0 * row.sales / sales } else { 0.0 },
                row.workers,
                row.crew.0,
                row.crew.1,
                row.input_stock,
            );
        }
        let record = city.record();
        println!("day {} record: {record:?}", city.core.time.day_index);
        previous = now;
    }
}

// The smallest zoned lot: one row of commercial cells along a street holds one-cell lots, which
// only the one-cell kiosk fits; each is built with its entrance on the street.
#[test]
#[ignore = "builds a street grid; run in a release test build"]
fn one_cell_lots_take_kiosks() {
    use crate::simulation::zoning::ZoneType;
    use crate::simulation::zoning::cells::CellSelectionShape;
    use glam::DVec2;
    let (mut core, row_z, half_x) = super::street_grid(400.0);
    super::super::scenario::install_fixture_assets(&mut core, true);
    // Lanes 3.5 m and sidewalks 1.5 m: the first cell row's centres stand 10 m off the centre
    // line; a 4 m brush there selects that row alone.
    let (x0, x1, z) = (
        f64::from(-half_x) + 60.0,
        f64::from(-half_x) + 160.0,
        f64::from(row_z[1]) + 10.0,
    );
    super::super::scenario::paint(
        &mut core,
        ZoneType::Commercial,
        CellSelectionShape::Brush { radius_m: 4.0 },
        &[DVec2::new(x0, z), DVec2::new(x1, z)],
    );
    super::fill_lots(&mut core);
    let kiosk = core
        .demand
        .runtime_catalog()
        .profile_for_id("grocery_kiosk")
        .unwrap()
        .runtime_id;
    let kiosks: Vec<_> = core
        .allocator
        .buildings
        .iter()
        .filter(|building| building.economy_profile_runtime_id == kiosk)
        .collect();
    println!("{} kiosks on one-cell lots", kiosks.len());
    assert!(kiosks.len() >= 5, "{} kiosks", kiosks.len());
    for building in kiosks {
        assert_eq!((building.width_cells, building.depth_cells), (1, 1));
        assert!(!building.broken && !building.economy_broken);
        assert_ne!(building.edge_idx, usize::MAX, "a kiosk has its street access");
    }
}

// The starter town grows from an empty main street through the demand planner, where the store
// size rule decides (`ECON-32`): its daily store census, with and without the kiosk asset.
//
// `METRUM_TEST_KIOSK=1 METRUM_KIOSK_TOWN_DAYS=120 $TEST_BINARY
// nodes::sim::core::populated_city::kiosk_mix::starter_town_store_mix --exact --ignored --nocapture`
#[test]
#[ignore = "grows the starter town; run in a release test build"]
fn starter_town_store_mix() {
    use super::super::scenario::GrowthScenario;
    let days = env_u32("METRUM_KIOSK_TOWN_DAYS", 120);
    let kiosk = std::env::var_os("METRUM_TEST_KIOSK").is_some();
    let mut town = GrowthScenario::starter_town();
    println!("starter_town_store_mix days={days} kiosk_asset={kiosk}");
    let mut previous = String::new();
    for _ in 0..days {
        let record = town.run_days(1)[0];
        let rows = census(town.core());
        let sales: f64 = rows.iter().map(|row| row.sales).sum();
        let line = rows
            .iter()
            .map(|row| {
                format!(
                    "{} {}+{} ({:.0}% of sales, {} workers)",
                    row.profile,
                    row.standing,
                    row.constructing,
                    if sales > 0.0 { 100.0 * row.sales / sales } else { 0.0 },
                    row.workers,
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let stores: String = rows
            .iter()
            .map(|row| format!("{}{}", row.standing, row.constructing))
            .collect();
        if stores != previous || town.day().is_multiple_of(10) {
            println!(
                "day {} residents {}: {line}",
                town.day(),
                record.residents
            );
        }
        previous = stores;
    }
}
