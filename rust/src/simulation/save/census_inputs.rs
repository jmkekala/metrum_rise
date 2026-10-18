// SPDX-License-Identifier: GPL-2.0-only

//! The funding census's input copy (`SIM-02-I`): its front, back, the building rows still to
//! copy again and the front rows a census defers, as blobs the copy encodes, with building
//! edges as saved edge ids; and the census in progress over the hour's second half, as the slot
//! that publishes it and whether it was current. A load recomputes a current census whole.
//! A save before the hour's actions slot also keeps the census the hour's first slot published,
//! which the hour's planning slots read: whether its books were valid and the copy it read.

use super::{SaveLoadError, SaveLoadResult, SnapshotMaps};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::demand::PlanningSlot;
use crate::simulation::economy::hiring::{
    CensusInputs, RollingCensus, SavedCensusInputs, SavedRollingCensus,
};
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::network::TransitNetwork;
use rusqlite::{Connection, Transaction, params};

pub(super) fn save(
    tx: &Transaction<'_>,
    households: &HouseholdSystem,
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
    time: &TimeSystem,
    maps: &SnapshotMaps,
) -> SaveLoadResult<()> {
    // A copied row may name an edge deleted since its copy; the copy refreshes that row
    // before reading it, so its saved edge is a placeholder.
    let edge = |edge| {
        maps.edge_old_to_new
            .get(&edge)
            .map_or(u64::MAX, |&saved| saved as u64)
    };
    let inputs = &households.census_inputs;
    let saved = inputs.save(&allocator.census_copy_changes, edge);
    let key = households.census_key(network);
    let census = households.census_run.save(key);
    // The hour's planning slots up to its actions slot read the census its first slot
    // published (`SIM-02-I`). Valid books mean the network and building indices are as they
    // were then, so no edge the copy names was deleted since.
    let published = (time.absolute_slot() % SLOTS_PER_HOUR < PlanningSlot::Actions as u64)
        .then(|| {
            let funding = &households.job_funding;
            let valid = funding
                .published
                .is_some_and(|published| published.books_valid_for(key));
            let covered = inputs.front_len().0;
            let adopted = funding
                .service_capacities
                .get(covered..)
                .unwrap_or_default();
            let saved = inputs.save_published(valid, adopted, edge);
            if saved.is_none() {
                crate::debug_log!(
                    "save",
                    "the published census has no kept copy; the load counts the loaded city"
                );
            }
            saved
        })
        .flatten();
    tx.execute(
        "INSERT INTO census_inputs(singleton, front, back, changes, deferred, \
         census_publish_slot, census_current, published) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            saved.front,
            saved.back,
            saved.changes,
            saved.deferred,
            census
                .map(|census| i64::try_from(census.publish_slot))
                .transpose()
                .map_err(|err| SaveLoadError::custom(err.to_string()))?,
            census.map(|census| census.current),
            published,
        ],
    )?;
    Ok(())
}

pub(super) fn load(conn: &Connection) -> SaveLoadResult<(CensusInputs, RollingCensus)> {
    let (saved, census) = conn.query_row(
        "SELECT front, back, changes, deferred, census_publish_slot, census_current, \
         published FROM census_inputs WHERE singleton = 1",
        [],
        |row| {
            Ok((
                SavedCensusInputs {
                    front: row.get(0)?,
                    back: row.get(1)?,
                    changes: row.get(2)?,
                    deferred: row.get(3)?,
                    published: row.get(6)?,
                },
                (
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<bool>>(5)?,
                ),
            ))
        },
    )?;
    let census = match census {
        (Some(slot), Some(current)) => Some(SavedRollingCensus {
            publish_slot: u64::try_from(slot)
                .map_err(|err| SaveLoadError::custom(err.to_string()))?,
            current,
        }),
        (None, None) => None,
        _ => {
            return Err(SaveLoadError::custom(
                "census in progress without its publishing slot",
            ));
        }
    };
    let inputs = CensusInputs::restore(&saved).map_err(SaveLoadError::custom)?;
    Ok((inputs, RollingCensus::restore(census)))
}
