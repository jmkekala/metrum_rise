// SPDX-License-Identifier: GPL-2.0-only

//! The funding census's input copy (`SIM-02-I`): its front, back, the building rows still to
//! copy again and the front rows a census defers, as blobs the copy encodes, with building
//! edges as saved edge ids; and the census in progress over the hour's second half, as the slot
//! that publishes it and whether it was current. A load recomputes a current census whole.

use super::{SaveLoadError, SaveLoadResult, SnapshotMaps};
use crate::simulation::buildings::allocator::BuildingAllocator;
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
    maps: &SnapshotMaps,
) -> SaveLoadResult<()> {
    // A copied row may name an edge deleted since its copy; the copy refreshes that row
    // before reading it, so its saved edge is a placeholder.
    let saved = households
        .census_inputs
        .save(&allocator.census_copy_changes, |edge| {
            maps.edge_old_to_new
                .get(&edge)
                .map_or(u64::MAX, |&saved| saved as u64)
        });
    let census = households.census_run.save(households.census_key(network));
    tx.execute(
        "INSERT INTO census_inputs(singleton, front, back, changes, deferred, \
         census_publish_slot, census_current) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
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
        ],
    )?;
    Ok(())
}

pub(super) fn load(conn: &Connection) -> SaveLoadResult<(CensusInputs, RollingCensus)> {
    let (saved, census) = conn.query_row(
        "SELECT front, back, changes, deferred, census_publish_slot, census_current \
         FROM census_inputs WHERE singleton = 1",
        [],
        |row| {
            Ok((
                SavedCensusInputs {
                    front: row.get(0)?,
                    back: row.get(1)?,
                    changes: row.get(2)?,
                    deferred: row.get(3)?,
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
