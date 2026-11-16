// SPDX-License-Identifier: GPL-2.0-only

//! Save determinism (`ROAD-68-K16`): a save is a function of the city and the save time it is
//! given, so a played town saves the same bytes twice, and the game loaded from that save saves
//! them again.

use super::GrowthScenario;
use super::census_cases::busy_town;
use crate::nodes::sim::core::SimCore;
use std::path::{Path, PathBuf};

// Any fixed header time: only its equality across the saves matters.
const SAVED_AT_UNIX: i64 = 1_761_436_800;

pub(super) fn save(core: &mut SimCore, path: &Path) {
    core.save_game_settled_at(path.to_str().unwrap(), None, SAVED_AT_UNIX)
        .unwrap();
}

// The tables whose rows, in storage order, differ between two saves: what a byte difference
// comes from.
pub(super) fn differing_tables(first: &Path, second: &Path) -> Vec<String> {
    let rows = |path: &Path| {
        let connection = rusqlite::Connection::open(path).unwrap();
        let tables: Vec<String> = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        tables
            .into_iter()
            .map(|table| {
                let mut statement = connection
                    .prepare(&format!("SELECT * FROM \"{table}\""))
                    .unwrap();
                let columns = statement.column_count();
                let rows: Vec<Vec<rusqlite::types::Value>> = statement
                    .query_map([], |row| {
                        (0..columns).map(|column| row.get(column)).collect()
                    })
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                (table, rows)
            })
            .collect::<Vec<_>>()
    };
    let (first, second) = (rows(first), rows(second));
    let mut tables: Vec<String> = first
        .iter()
        .filter(|table| !second.contains(table))
        .map(|(table, _)| table.clone())
        .collect();
    if first.len() != second.len() {
        tables.push("(table count)".into());
    }
    tables
}

fn assert_same_bytes(first: &Path, second: &Path, what: &str) {
    assert!(
        std::fs::read(first).unwrap() == std::fs::read(second).unwrap(),
        "{what}: tables differing in storage order: {:?}",
        differing_tables(first, second)
    );
}

#[test]
fn a_town_saves_the_same_bytes_twice_and_after_a_load() {
    let path = |name: &str| -> PathBuf {
        std::env::temp_dir().join(format!(
            "metrum-save-bytes-{name}-{}.sqlite",
            std::process::id()
        ))
    };
    let (first, second, reloaded) = (path("first"), path("second"), path("reloaded"));
    // Eight days of play: households, jobs, shipments in transit, lanes and their connections
    // at every junction of the town's streets.
    let mut town = busy_town();
    save(&mut town.core, &first);
    save(&mut town.core, &second);
    assert_same_bytes(&first, &second, "a second save of the town");
    let mut loaded = GrowthScenario::starter_town();
    loaded
        .core
        .load_game_internal(first.to_str().unwrap())
        .unwrap();
    // A load pauses the game; the town was saved running.
    loaded.core.time.speed_multiplier = town.core.time.speed_multiplier;
    save(&mut loaded.core, &reloaded);
    assert_same_bytes(&first, &reloaded, "the save of the game loaded from it");
    for path in [first, second, reloaded] {
        std::fs::remove_file(path).unwrap();
    }
}

// `SAVE-06`: bulldozing a town's only border road while freight is on its way in and out cuts
// the border's trade. Its shipments settle at the edit as a removed building's do: an import
// in transit refunds its buyer, a loaded export's cargo goes back to its source, the carriers
// leave and closed shipments drop, so no shipment names the border node, which the save no
// longer keeps. The town saves and loads, and the loaded game's freight runs on as the live
// game's; an undo brings the road back, trade over it resumes and the town saves and loads
// again. Before, closed and open shipments kept the border node and every save failed with
// "missing shipment border-node mapping" once no agent stood on the node.
#[test]
fn bulldozing_the_border_road_settles_its_freight_and_the_town_saves() {
    use super::super::frame::FRAME_DT_S;
    use crate::simulation::economy::logistics::{Shipment, ShipmentEndpoint, ShipmentStatus};
    use crate::simulation::network::types::NodeType;

    let path = |name: &str| -> PathBuf {
        std::env::temp_dir().join(format!(
            "metrum-save06-{name}-{}.sqlite",
            std::process::id()
        ))
    };
    let mut town = busy_town();
    let core = &mut town.core;
    // A side road splits the trunk 400 m short of the town, so the border's road fronts no
    // building: a building on a removed road keeps its edge (`SAVE-07`).
    super::commit_road(core, (-600.0, -60.0), (-600.0, 60.0));
    let graph = &core.region_graph;
    let border = (0..graph.node_count() as u32)
        .find(|&node| {
            graph.node(node).node_type == NodeType::Border
                && graph.node_has_live_incident_edge(node)
        })
        .unwrap();
    let at_border = ShipmentEndpoint::OwaBorder(border);
    let names_border =
        |shipment: &Shipment| shipment.source == at_border || shipment.destination == at_border;
    // An open shipment into (`inbound`) or out of the town over the border; `loaded` asks for
    // one whose cargo is under way.
    let open = |core: &SimCore, inbound: bool, loaded: bool| {
        core.logistics.shipments.iter().any(|shipment| {
            shipment.status.is_open()
                && (!loaded
                    || shipment.status == ShipmentStatus::InTransit && shipment.cargo_loaded)
                && if inbound {
                    shipment.source == at_border
                } else {
                    shipment.destination == at_border
                }
        })
    };
    let end = core.time.day_index + 3;
    let both = |core: &SimCore| open(core, true, true) && open(core, false, true);
    while !both(core) && core.time.day_index < end {
        core.step_frame(FRAME_DT_S);
    }
    assert!(both(core), "imports and exports under way");
    let graph = &core.region_graph;
    let road: Vec<usize> = graph
        .node_adjacency(border)
        .iter()
        .copied()
        .filter(|&edge| !graph.edge(edge).deleted)
        .collect();
    assert_eq!(road.len(), 1, "one road reaches the border");
    assert!(
        core.allocator
            .buildings
            .iter()
            .all(|building| building.edge_idx != road[0]),
        "no building fronts the border's road"
    );

    // What the settlement owes: each import buyer's reserved payment, each loaded export's cargo.
    let touched: Vec<Shipment> = core
        .logistics
        .shipments
        .iter()
        .filter(|shipment| names_border(shipment))
        .cloned()
        .collect();
    let mut owed_budget = std::collections::BTreeMap::<usize, f64>::new();
    let mut owed_cargo = std::collections::BTreeMap::<(usize, u16), f64>::new();
    for shipment in &touched {
        match (shipment.source, shipment.destination) {
            (ShipmentEndpoint::OwaBorder(_), ShipmentEndpoint::Building(buyer))
                if shipment.status == ShipmentStatus::InTransit =>
            {
                *owed_budget.entry(buyer).or_default() += f64::from(shipment.total_cost);
            }
            (ShipmentEndpoint::Building(seller), ShipmentEndpoint::OwaBorder(_))
                if shipment.status == ShipmentStatus::InTransit && shipment.cargo_loaded =>
            {
                *owed_cargo
                    .entry((seller, shipment.resource_runtime_id))
                    .or_default() += f64::from(shipment.amount);
            }
            _ => {}
        }
    }
    let allocator = &core.allocator;
    assert!(
        owed_budget
            .keys()
            .all(|&buyer| !allocator.is_city_service_building(&allocator.buildings[buyer])),
        "the buyers pay from their own budgets"
    );
    let budget =
        |core: &SimCore, buyer: usize| f64::from(core.allocator.buildings[buyer].operating_budget);
    let cargo = |core: &SimCore, (seller, resource): (usize, u16)| {
        f64::from(core.allocator.buildings[seller].inventory_units(resource))
    };
    let budgets: Vec<f64> = owed_budget
        .keys()
        .map(|&buyer| budget(core, buyer))
        .collect();
    let cargoes: Vec<f64> = owed_cargo.keys().map(|&key| cargo(core, key)).collect();
    let buildings = core.allocator.buildings.len();

    assert!(core.bulldoze_road_edge_for_test(road[0]));
    assert!(!core.region_graph.node_has_live_incident_edge(border));
    assert_eq!(core.allocator.buildings.len(), buildings);
    let names_no_cut_border = |core: &SimCore| {
        let left: Vec<u64> = core
            .logistics
            .shipments
            .iter()
            .filter(|shipment| names_border(shipment))
            .map(|shipment| shipment.id)
            .collect();
        assert!(left.is_empty(), "shipments naming the cut border: {left:?}");
    };
    names_no_cut_border(core);
    assert!(
        (0..core.agents.len()).all(|i| touched
            .iter()
            .all(|shipment| !core.agents.freight_carrier_matches(i, shipment.id))),
        "the settled shipments' carriers left"
    );
    for ((&buyer, &owed), before) in owed_budget.iter().zip(budgets) {
        assert!(
            (budget(core, buyer) - before - owed).abs() < 1e-2,
            "buyer {buyer} is refunded {owed}"
        );
    }
    for ((&key, &owed), before) in owed_cargo.iter().zip(cargoes) {
        assert!(
            (cargo(core, key) - before - owed).abs() < 1e-3,
            "seller {key:?} has its {owed} back"
        );
    }

    // The town saves at once and after it ran on. The game loaded from the first save runs its
    // freight as the live game does, frame by frame, and both save the same freight rows. (The
    // whole state does not continue alike after this edit: `SAVE-08`, `SAVE-09`.)
    let (cut, ran, reloaded) = (path("cut"), path("ran"), path("reloaded"));
    save(core, &cut);
    let mut loaded = GrowthScenario::starter_town();
    loaded
        .core
        .load_game_internal(cut.to_str().unwrap())
        .unwrap();
    loaded.core.time.speed_multiplier = core.time.speed_multiplier;
    type Freight = (Vec<String>, Vec<(usize, u16, u16, bool)>, u64, u64);
    let freight = |core: &SimCore| -> Freight {
        let logistics = &core.logistics;
        (
            logistics
                .shipments
                .iter()
                .map(|shipment| format!("{shipment:?}"))
                .collect(),
            logistics
                .request_failures
                .iter()
                .map(|(key, failure)| {
                    let (building, resource) =
                        (key.destination_building_id, key.resource_runtime_id);
                    (building, resource, failure.failures, failure.terminal)
                })
                .collect(),
            logistics.next_shipment_id,
            core.treasury.balance.to_bits(),
        )
    };
    for frame in 0..600 {
        core.step_frame(FRAME_DT_S);
        loaded.core.step_frame(FRAME_DT_S);
        assert_eq!(
            freight(core),
            freight(&loaded.core),
            "frame {frame} after the load"
        );
    }
    names_no_cut_border(core);
    save(core, &ran);
    save(&mut loaded.core, &reloaded);
    let freight_tables = differing_tables(&ran, &reloaded);
    assert!(
        ["shipments", "freight_request_failures", "freight_requests"]
            .iter()
            .all(|table| !freight_tables.iter().any(|differing| differing == table)),
        "{freight_tables:?}"
    );

    // Undo brings the road back; imports cross the border again and the town saves and loads.
    assert!(core.undo_action_internal());
    assert!(core.region_graph.node_has_live_incident_edge(border));
    let end = core.time.day_index + 2;
    while !open(core, true, false) && core.time.day_index < end {
        core.step_frame(FRAME_DT_S);
    }
    assert!(open(core, true, false), "imports cross the restored border");
    let restored = path("restored");
    save(core, &restored);
    GrowthScenario::starter_town()
        .core
        .load_game_internal(restored.to_str().unwrap())
        .unwrap();
    for path in [cut, ran, reloaded, restored] {
        std::fs::remove_file(path).unwrap();
    }
}

// `SAVE-07`: bulldozing a road that buildings front removes them in the same edit, as the stale
// building cleanup would at their next daily phase, so no building names the removed edge and
// the town saves at once, after a few frames and after an undo, and every save loads. The undo
// brings the road back but not the buildings (a road undo returns paint, never buildings,
// `ZONE-09`). Before, the buildings kept the removed edge until their daily phase and a save
// in between failed with "bld edge missing".
#[test]
fn bulldozing_a_road_buildings_front_removes_them_and_the_town_saves() {
    use super::super::frame::FRAME_DT_S;

    let path = |name: &str| -> PathBuf {
        std::env::temp_dir().join(format!(
            "metrum-save07-{name}-{}.sqlite",
            std::process::id()
        ))
    };
    let mut town = busy_town();
    let core = &mut town.core;
    // The road most buildings front, the lowest id on a tie.
    let fronting = |core: &SimCore, edge: usize| {
        core.allocator
            .buildings
            .iter()
            .filter(|building| building.edge_idx == edge)
            .count()
    };
    let graph = &core.region_graph;
    let road = (0..graph.edge_count())
        .filter(|&edge| !graph.edge(edge).deleted)
        .max_by_key(|&edge| (fronting(core, edge), std::cmp::Reverse(edge)))
        .unwrap();
    let fronted = fronting(core, road);
    assert!(fronted > 0, "a road buildings front");
    let buildings = core.allocator.buildings.len();
    let no_removed_edge = |core: &SimCore| {
        let graph = &core.region_graph;
        let stale: Vec<(usize, usize)> = core
            .allocator
            .buildings
            .iter()
            .enumerate()
            .filter(|(_, building)| {
                graph
                    .get_edge(building.edge_idx)
                    .is_none_or(|edge| edge.deleted)
            })
            .map(|(index, building)| (index, building.edge_idx))
            .collect();
        assert!(
            stale.is_empty(),
            "buildings naming a removed edge: {stale:?}"
        );
    };

    assert!(core.bulldoze_road_edge_for_test(road));
    assert!(core.region_graph.edge(road).deleted);
    no_removed_edge(core);
    let left = core.allocator.buildings.len();
    assert!(left < buildings, "the road's buildings go with it");
    assert!(left >= buildings - fronted);

    // The town saves at once and a few frames on; each save loads with the same buildings.
    let loads = |core: &SimCore, saved: &Path| {
        let mut loaded = GrowthScenario::starter_town();
        loaded
            .core
            .load_game_internal(saved.to_str().unwrap())
            .unwrap();
        assert_eq!(
            loaded.core.allocator.buildings.len(),
            core.allocator.buildings.len()
        );
        no_removed_edge(&loaded.core);
    };
    let (cut, ran, restored) = (path("cut"), path("ran"), path("restored"));
    save(core, &cut);
    loads(core, &cut);
    for _ in 0..5 {
        core.step_frame(FRAME_DT_S);
    }
    no_removed_edge(core);
    save(core, &ran);
    loads(core, &ran);

    // Undo brings the road back, not its buildings; the town saves and loads.
    assert!(core.undo_action_internal());
    assert!(!core.region_graph.edge(road).deleted);
    no_removed_edge(core);
    assert_eq!(core.allocator.buildings.len(), left);
    save(core, &restored);
    loads(core, &restored);
    for path in [cut, ran, restored] {
        std::fs::remove_file(path).unwrap();
    }
}
