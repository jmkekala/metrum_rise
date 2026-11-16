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
    // whole state does not continue alike after this edit: `SAVE-11`.)
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

// `ROAD-122`: undoing a border connection cuts the border's trade as removing its road does
// (`SAVE-06`): the shipments naming it settle in the undo, none names it while the town runs on,
// and the town saves and loads. The town's own connection was made in its setup, under later
// records; the test makes it again on top of the history, with its freight under way.
#[test]
fn undoing_a_border_connection_settles_its_freight_and_the_town_saves() {
    use super::super::frame::FRAME_DT_S;
    use crate::simulation::economy::logistics::ShipmentEndpoint;
    use crate::simulation::network::types::NodeType;

    let mut town = busy_town();
    let core = &mut town.core;
    let graph = &core.region_graph;
    let border = (0..graph.node_count() as u32)
        .find(|&node| {
            graph.node(node).node_type == NodeType::Border
                && graph.node_has_live_incident_edge(node)
        })
        .unwrap();
    let at_border = ShipmentEndpoint::OwaBorder(border);
    let named = |core: &SimCore| {
        core.logistics
            .shipments
            .iter()
            .filter(|shipment| shipment.source == at_border || shipment.destination == at_border)
            .count()
    };
    let end = core.time.day_index + 3;
    while named(core) == 0 && core.time.day_index < end {
        core.step_frame(FRAME_DT_S);
    }
    assert!(named(core) > 0, "freight crosses the border");
    core.region_graph.set_node_type(border, NodeType::Junction);
    core.set_border_connection_internal(border as i32);
    assert_eq!(core.region_graph.node(border).node_type, NodeType::Border);
    assert!(named(core) > 0, "the connection settles nothing");

    assert!(core.undo_action_internal());
    assert_eq!(core.region_graph.node(border).node_type, NodeType::Junction);
    assert_eq!(named(core), 0, "the undo settles the border's freight");
    for _ in 0..600 {
        core.step_frame(FRAME_DT_S);
    }
    assert_eq!(named(core), 0, "no new freight crosses the former border");
    let path = std::env::temp_dir().join(format!(
        "metrum-road122-border-undo-{}.sqlite",
        std::process::id()
    ));
    save(core, &path);
    GrowthScenario::starter_town()
        .core
        .load_game_internal(path.to_str().unwrap())
        .unwrap();
    std::fs::remove_file(path).unwrap();
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

// The saved request failures as rows: building, resource, failures, terminal.
fn request_failures(core: &SimCore) -> Vec<(usize, u16, u16, bool)> {
    core.logistics
        .request_failures
        .iter()
        .map(|(key, failure)| {
            let (building, resource) = (key.destination_building_id, key.resource_runtime_id);
            (building, resource, failure.failures, failure.terminal)
        })
        .collect()
}

// `SAVE-08`: a town whose border road is gone fails freight requests; a save keeps the failures
// and whether the freight route cache was valid, so the game loaded from it keeps them through
// its first route refresh (the load's own), as the running game does until a real routing
// change, and saves them again. Before, the load restored no cache revisions, so its first
// refresh took the load for a topology change and cleared the failures.
#[test]
fn freight_request_failures_survive_a_save_and_load() {
    use super::super::frame::FRAME_DT_S;
    use crate::simulation::network::types::NodeType;

    let path = |name: &str| -> PathBuf {
        std::env::temp_dir().join(format!(
            "metrum-save08-{name}-{}.sqlite",
            std::process::id()
        ))
    };
    let mut town = busy_town();
    let core = &mut town.core;
    // The border road keeps no building (`SAVE-07`).
    super::commit_road(core, (-600.0, -60.0), (-600.0, 60.0));
    let graph = &core.region_graph;
    let border = (0..graph.node_count() as u32)
        .find(|&node| {
            graph.node(node).node_type == NodeType::Border
                && graph.node_has_live_incident_edge(node)
        })
        .unwrap();
    let road = graph
        .node_adjacency(border)
        .iter()
        .copied()
        .find(|&edge| !graph.edge(edge).deleted)
        .unwrap();
    assert!(core.bulldoze_road_edge_for_test(road));
    let end = core.time.day_index + 2;
    while core.logistics.request_failures.is_empty() && core.time.day_index < end {
        core.step_frame(FRAME_DT_S);
    }
    let failures = request_failures(core);
    assert!(!failures.is_empty(), "a request failed without a border");
    assert_ne!(
        core.logistics
            .route_cache_validity(&core.allocator, &core.transit_network),
        0,
        "the running game's next refresh keeps its failures"
    );

    let (saved, resaved) = (path("saved"), path("resaved"));
    save(core, &saved);
    let mut loaded = GrowthScenario::starter_town();
    loaded
        .core
        .load_game_internal(saved.to_str().unwrap())
        .unwrap();
    loaded.core.time.speed_multiplier = core.time.speed_multiplier;
    assert_eq!(request_failures(&loaded.core), failures, "at the load");
    save(&mut loaded.core, &resaved);
    let differing = differing_tables(&saved, &resaved);
    assert!(
        !differing
            .iter()
            .any(|table| table == "freight_request_failures"),
        "{differing:?}"
    );
    // Both games' freight refreshes keep the failures alike while their routing is unchanged.
    for frame in 0..120 {
        core.step_frame(FRAME_DT_S);
        loaded.core.step_frame(FRAME_DT_S);
        assert_eq!(
            request_failures(&loaded.core),
            request_failures(core),
            "frame {frame} after the load"
        );
    }
    for path in [saved, resaved] {
        std::fs::remove_file(path).unwrap();
    }
}

// `SAVE-09`: the day's construction before a save and the buildings' daily input telemetry,
// which the census copies, continue in the game loaded from it: the next ledger entry charges the
// same construction and every building keeps its day's input values. Before, the load took the
// loaded lifetime build cost as the ledger's baseline and zeroed the telemetry. A format-119
// save, which has neither, loads as before.
#[test]
fn the_days_construction_and_input_telemetry_survive_a_save_and_load() {
    use crate::simulation::save::schema::PRE_CONTINUATION_SAVE_VERSION;

    let path = |name: &str| -> PathBuf {
        std::env::temp_dir().join(format!(
            "metrum-save09-{name}-{}.sqlite",
            std::process::id()
        ))
    };
    let mut town = busy_town();
    let core = &mut town.core;
    super::commit_road(core, (-600.0, -60.0), (-600.0, 60.0));
    assert!(
        core.treasury.lifetime_build_cost > core.treasury.ledger_build_cost,
        "the road is the day's construction"
    );
    let telemetry = |core: &SimCore| -> Vec<(u32, u32)> {
        core.allocator
            .buildings
            .iter()
            .map(|building| {
                (
                    building.daily_owa_input_value.to_bits(),
                    building.daily_local_input_value.to_bits(),
                )
            })
            .collect()
    };
    let inputs = telemetry(core);
    assert!(
        inputs.iter().any(|&(owa, local)| owa != 0 || local != 0),
        "the day's input purchases"
    );

    let saved = path("saved");
    save(core, &saved);
    let mut loaded = GrowthScenario::starter_town();
    loaded
        .core
        .load_game_internal(saved.to_str().unwrap())
        .unwrap();
    assert_eq!(
        loaded.core.treasury.ledger_build_cost.to_bits(),
        core.treasury.ledger_build_cost.to_bits()
    );
    assert_eq!(telemetry(&loaded.core), inputs);
    // The next entry, as the day's close records it.
    let day = core.time.day_index;
    let construction = |core: &mut SimCore| {
        core.record_daily_budget_ledger(day);
        core.budget_history
            .back()
            .unwrap()
            .construction_service_costs
    };
    let running = construction(core);
    assert_eq!(construction(&mut loaded.core).to_bits(), running.to_bits());

    // The same save in format 119: the ledger starts at the load and the telemetry at zero.
    rusqlite::Connection::open(&saved)
        .unwrap()
        .execute_batch(&format!(
            "ALTER TABLE city_treasury DROP COLUMN ledger_build_cost;
             ALTER TABLE buildings DROP COLUMN daily_owa_input_value;
             ALTER TABLE buildings DROP COLUMN daily_local_input_value;
             ALTER TABLE scheduler_state DROP COLUMN freight_route_validity;
             UPDATE save_meta SET version = {PRE_CONTINUATION_SAVE_VERSION};"
        ))
        .unwrap();
    let mut legacy = GrowthScenario::starter_town();
    legacy
        .core
        .load_game_internal(saved.to_str().unwrap())
        .unwrap();
    let treasury = &legacy.core.treasury;
    assert_eq!(
        treasury.ledger_build_cost.to_bits(),
        treasury.lifetime_build_cost.to_bits()
    );
    assert!(
        telemetry(&legacy.core)
            .iter()
            .all(|&values| values == (0, 0))
    );
    std::fs::remove_file(saved).unwrap();
}

// The road graph and lanes of a loaded game equal the saved game's in memory (`SAVE-11`): node,
// edge and lane ids are saved state. Every node slot resolves to the same node and every
// canonical one with a road has its position and type; every edge slot is live or a hole in
// both, a live one with its whole record; every lane slot is a lane of the same owner with the
// same geometry, successors, delays and ring entry, or a retired lane in both. Positions of
// merged nodes and the records of deleted edges are no state anyone reads, and not compared.
pub(in crate::nodes::sim::core) fn assert_same_network(live: &SimCore, loaded: &SimCore, what: &str) {
    let (a, b) = (&live.region_graph, &loaded.region_graph);
    assert_eq!(a.node_count(), b.node_count(), "{what}: node count");
    assert_eq!(a.edge_count(), b.edge_count(), "{what}: edge count");
    for node in 0..a.node_count() as u32 {
        let canonical = a.get_valid_node(node);
        assert_eq!(canonical, b.get_valid_node(node), "{what}: node {node}'s canonical node");
        if canonical == node {
            let (x, y) = (a.node(node), b.node(node));
            assert_eq!(
                (x.pos, x.node_type, &x.crosswalk_overrides),
                (y.pos, y.node_type, &y.crosswalk_overrides),
                "{what}: node {node}"
            );
            assert_eq!(
                a.node_adjacency(node),
                b.node_adjacency(node),
                "{what}: node {node}'s roads"
            );
        }
    }
    for edge in 0..a.edge_count() {
        let (x, y) = (a.edge(edge), b.edge(edge));
        assert_eq!(x.deleted, y.deleted, "{what}: edge {edge} deleted");
        if x.deleted {
            continue;
        }
        let record = |e: &crate::simulation::network::graph::Edge| {
            format!(
                "{} {} {:?} {} {:?} {} {} {} {} {} {} {} {} {:?} {:?} {} {} {:?} {:?} {:?} {:?} {:?} {:?}",
                e.start_node,
                e.end_node,
                e.primary_type,
                e.allowed_types,
                e.class,
                e.width.to_bits(),
                e.fwd_lanes,
                e.bkw_lanes,
                e.speed_limit.to_bits(),
                e.base_cost.to_bits(),
                e.physical_length.to_bits(),
                e.start_clip.to_bits(),
                e.end_clip.to_bits(),
                e.geometry,
                e.physical_geometry,
                e.no_building_spawn,
                e.no_street_lights,
                e.vehicle_frontage_access,
                e.median,
                e.parking,
                e.pocket,
                e.sidewalks,
                e.roundabout,
            )
        };
        assert_eq!(record(x), record(y), "{what}: edge {edge}");
    }
    let (a, b) = (
        &live.transit_network.lane_system,
        &loaded.transit_network.lane_system,
    );
    assert_eq!(a.lanes.len(), b.lanes.len(), "{what}: lane count");
    assert_eq!(a.edge_lanes, b.edge_lanes, "{what}: road lanes");
    assert_eq!(a.node_lanes, b.node_lanes, "{what}: connectors");
    for id in 0..a.lanes.len() {
        let lane = |lanes: &crate::simulation::network::lanes::LaneSystem| {
            let l = &lanes.lanes[id];
            format!(
                "{} {} {} {} {:?} {:?} {:?} {:?} {} {} {} {} {} {:?}",
                l.edge_id,
                l.node_id,
                l.is_fwd,
                l.lane_idx,
                l.lane_type,
                &*l.next_lanes,
                l.geometry,
                l.cum_dist,
                l.length.to_bits(),
                l.frontage_delay_penalty_s.to_bits(),
                l.live_frontage_delay_s.to_bits(),
                l.route_delay_s.to_bits(),
                l.solid_zone_m.to_bits(),
                lanes.ring_entry(id),
            )
        };
        assert_eq!(lane(a), lane(b), "{what}: lane {id}");
    }
}

// Every agent's movement state, its lane ids included, as bits.
fn agent_states(core: &SimCore) -> Vec<[u64; 12]> {
    let a = &core.agents.agents;
    (0..a.len())
        .map(|i| {
            [
                u64::from(a.pos_x[i].to_bits()),
                u64::from(a.pos_y[i].to_bits()),
                u64::from(a.speed[i].to_bits()),
                u64::from(a.lane_distance[i].to_bits()),
                u64::from(a.activity[i]),
                u64::from(a.transit[i]),
                a.current_lane_id[i] as u64,
                a.current_edge[i] as u64,
                u64::from(a.current_node[i]),
                u64::from(a.lane_change_from_lane_id[i]),
                u64::from(a.planned_attach_lane_id[i]),
                u64::from(a.planned_detach_lane_id[i]),
            ]
        })
        .collect()
}

// Saves `core`, loads the save into a fresh starter town and checks the loaded network equals
// the saved one in memory; the loaded game runs at the saved game's speed.
pub(in crate::nodes::sim::core) fn save_and_load(core: &mut SimCore, path: &Path) -> SimCore {
    save(core, path);
    let mut loaded = GrowthScenario::starter_town();
    loaded
        .core
        .load_game_internal(path.to_str().unwrap())
        .unwrap();
    loaded.core.time.speed_multiplier = core.time.speed_multiplier;
    assert_same_network(core, &loaded.core, "at the load");
    loaded.core
}

// Steps the saved and the loaded game together for `frames` frames: every agent's state, lane
// ids included, equal after every frame, and the two games' raw saves byte-equal every 300
// frames (no canonicalising load between them).
pub(in crate::nodes::sim::core) fn run_alike(
    live: &mut SimCore,
    loaded: &mut SimCore,
    frames: u32,
    name: &str,
) {
    use super::super::frame::FRAME_DT_S;
    let path = |game: &str| -> PathBuf {
        std::env::temp_dir().join(format!(
            "metrum-save11-{name}-{game}-{}.sqlite",
            std::process::id()
        ))
    };
    let (running, resumed) = (path("running"), path("loaded"));
    for frame in 1..=frames {
        live.step_frame(FRAME_DT_S);
        loaded.step_frame(FRAME_DT_S);
        let (a, b) = (agent_states(live), agent_states(loaded));
        assert_eq!(a.len(), b.len(), "frame {frame}: agent count");
        if let Some(agent) = (0..a.len()).find(|&i| a[i] != b[i]) {
            panic!(
                "frame {frame}: agent {agent} {:?} in the running game, {:?} loaded",
                a[agent], b[agent]
            );
        }
        if frame % 300 == 0 {
            save(live, &running);
            save(loaded, &resumed);
            assert_same_bytes(&running, &resumed, &format!("frame {frame}"));
        }
    }
    for path in [running, resumed] {
        std::fs::remove_file(path).unwrap();
    }
}

// `SAVE-11` (round S2): node, edge and lane ids are saved state, so a game saved after road
// edits loads with the running game's ids exactly and continues as it. The busy starter town
// takes the edits that renumbered a load before: a commit that splits the trunk's border piece,
// the bulldozing of the lowest road (the border road, leaving the lowest node without a road),
// an in-place class change and its undo, and a new road connected to the world's edge. The
// loaded network equals the running one in memory, and both games then run two game hours
// with every agent alike each frame and byte-equal saves every 300 frames. Before, the save
// compacted nodes and edges and a load numbered lanes anew: the loaded game's walkers broke
// ties on other ids (`SAVE-09`) and its saves wrote other lane ids.
#[test]
fn a_town_continues_across_a_save_after_road_edits_as_the_running_game() {
    use crate::simulation::network::types::NodeType;
    use godot::prelude::Vector3;

    let mut town = busy_town();
    let core = &mut town.core;
    let (nodes, edges) = (core.region_graph.node_count(), core.region_graph.edge_count());
    // A street across the trunk's border piece splits it.
    super::commit_road(core, (-600.0, -60.0), (-600.0, 60.0));
    let graph = &core.region_graph;
    let border = (0..graph.node_count() as u32)
        .find(|&node| {
            graph.node(node).node_type == NodeType::Border && graph.node_has_live_incident_edge(node)
        })
        .unwrap();
    let road = graph
        .node_adjacency(border)
        .iter()
        .copied()
        .find(|&edge| !graph.edge(edge).deleted)
        .unwrap();
    assert_eq!((border, road), (0, 0), "the lowest node and edge");
    assert!(core.bulldoze_road_edge_for_test(road));
    assert!(!core.region_graph.node_has_live_incident_edge(border));
    // An in-place class change on the new street's north half, then its undo.
    let north = core.get_hovered_edge_internal(-600.0, 40.0);
    assert!(north >= 0);
    assert!(core.set_edge_class_internal(north, 1));
    assert!(core.undo_action_internal());
    // A new road to the world's north edge, connected there.
    let (_, half_z) = core.heightmap.half_world_extents();
    let edge_z = half_z - 1.0;
    super::commit_road(core, (-600.0, 60.0), (-600.0, edge_z));
    let y = core.heightmap.sample_height_world(-600.0, edge_z);
    let end = core.check_border_candidate_internal(Vector3::new(-600.0, y, edge_z));
    assert!(end >= 0, "the new road ends at the world's edge");
    core.set_border_connection_internal(end as i32);
    let graph = &core.region_graph;
    assert!(graph.node_count() > nodes && graph.edge_count() > edges);
    let lanes = &core.transit_network.lane_system;
    assert!(
        lanes
            .lanes
            .iter()
            .any(|lane| lane.edge_id == usize::MAX && lane.node_id == usize::MAX),
        "the edits retired lanes"
    );

    let path = std::env::temp_dir().join(format!("metrum-save11-town-{}.sqlite", std::process::id()));
    let mut loaded = save_and_load(core, &path);
    std::fs::remove_file(path).unwrap();
    let start = core.time.absolute_slot();
    run_alike(core, &mut loaded, 2 * 7_200, "town");
    let hours = (core.time.absolute_slot() - start) as f64
        / crate::simulation::core::scheduler::SLOTS_PER_HOUR as f64;
    assert!(hours >= 2.0, "{hours} game hours");
}
