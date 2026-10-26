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

fn save(core: &mut SimCore, path: &Path) {
    core.save_game_settled_at(path.to_str().unwrap(), None, SAVED_AT_UNIX)
        .unwrap();
}

// The tables whose rows, in storage order, differ between two saves: what a byte difference
// comes from.
fn differing_tables(first: &Path, second: &Path) -> Vec<String> {
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
