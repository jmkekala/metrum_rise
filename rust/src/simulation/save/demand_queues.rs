// SPDX-License-Identifier: GPL-2.0-only

//! The demand request queues (`SIM-02-J`): the level changes the hour's actions slot proved,
//! with their permits; the queued building removals; the admitted households not created yet,
//! with the border each arrives from as a saved node id; and the admission credit. Each queue is
//! saved in order, so a loaded game serves the same requests at the same slots.

use super::{
    NONE_REF, SaveLoadError, SaveLoadResult, SnapshotMaps, handle_from_db, handle_to_db,
    i64_to_u16, i64_to_u32, i64_to_u64, u64_to_i64, usize_to_i64,
};
use crate::simulation::buildings::allocator::{
    BuildingAllocator, BuildingRemovalKind, BuildingRemovalRequest,
};
use crate::simulation::economy::agents::HouseholdAgeComposition;
use crate::simulation::economy::demand::{DemandSystem, QueuedLevelChange};
use crate::simulation::economy::households::{HouseholdSystem, QueuedAdmission};
use crate::simulation::network::graph::RegionGraph;
use rusqlite::{Connection, Transaction, params};

pub(super) fn save(
    tx: &Transaction<'_>,
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
    demand: &DemandSystem,
    graph: &RegionGraph,
    maps: &SnapshotMaps,
) -> SaveLoadResult<()> {
    let mut stmt = tx.prepare(
        "INSERT INTO level_change_requests(sequence, slot, building_slot, building_generation, target_asset_id, permit) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for (sequence, request) in demand.level_change_requests.iter().enumerate() {
        let (slot, generation) = handle_to_db(request.building);
        stmt.execute(params![
            usize_to_i64(sequence)?,
            u64_to_i64(request.slot)?,
            slot,
            generation,
            &request.target_asset_id,
            request.permit as i64,
        ])?;
    }
    let mut stmt = tx.prepare(
        "INSERT INTO building_removal_requests(sequence, slot, building_slot, building_generation, kind) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for (sequence, request) in allocator.removal_requests.iter().enumerate() {
        let (slot, generation) = handle_to_db(request.building);
        stmt.execute(params![
            usize_to_i64(sequence)?,
            u64_to_i64(request.slot)?,
            slot,
            generation,
            request.kind.to_saved(),
        ])?;
    }
    // A border no saved road reaches any more is saved as none; the loaded game then drops
    // the admission at its creation, as the running game finds the border without a road.
    let mut stmt = tx.prepare(
        "INSERT INTO admission_requests(sequence, home_slot, home_generation, household_size, child_count, adult_count, elder_count, border_node) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for (sequence, admission) in households.admission_requests.iter().enumerate() {
        let (slot, generation) = handle_to_db(admission.home);
        let border = ((admission.border_node as usize) < graph.node_count())
            .then(|| graph.get_valid_node(admission.border_node))
            .and_then(|node| maps.node_old_to_new.get(&node))
            .map_or(NONE_REF, |&saved| i64::from(saved));
        let composition = admission.composition;
        stmt.execute(params![
            usize_to_i64(sequence)?,
            slot,
            generation,
            i64::from(admission.size),
            i64::from(composition.child_count),
            i64::from(composition.adult_count),
            i64::from(composition.elder_count),
            border,
        ])?;
    }
    Ok(())
}

pub(super) fn load(
    conn: &Connection,
    allocator: &mut BuildingAllocator,
    households: &mut HouseholdSystem,
    demand: &mut DemandSystem,
) -> SaveLoadResult<()> {
    let mut stmt = conn.prepare(
        "SELECT slot, building_slot, building_generation, target_asset_id, permit FROM level_change_requests ORDER BY sequence",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        demand.level_change_requests.push_back(QueuedLevelChange {
            slot: i64_to_u64(row.get(0)?)?,
            building: handle_from_db(row.get(1)?, row.get(2)?)?,
            target_asset_id: row.get(3)?,
            permit: row.get::<_, i64>(4)? as u64,
        });
    }
    let mut stmt = conn.prepare(
        "SELECT slot, building_slot, building_generation, kind FROM building_removal_requests ORDER BY sequence",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        allocator
            .removal_requests
            .push_back(BuildingRemovalRequest {
                slot: i64_to_u64(row.get(0)?)?,
                building: handle_from_db(row.get(1)?, row.get(2)?)?,
                kind: BuildingRemovalKind::from_saved(row.get(3)?)
                    .ok_or_else(|| SaveLoadError::custom("unknown building removal kind"))?,
            });
    }
    let mut stmt = conn.prepare(
        "SELECT home_slot, home_generation, household_size, child_count, adult_count, elder_count, border_node FROM admission_requests ORDER BY sequence",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let border: i64 = row.get(6)?;
        households.admission_requests.push_back(QueuedAdmission {
            home: handle_from_db(row.get(0)?, row.get(1)?)?,
            size: i64_to_u16(row.get(2)?)?,
            composition: HouseholdAgeComposition {
                child_count: i64_to_u16(row.get(3)?)?,
                adult_count: i64_to_u16(row.get(4)?)?,
                elder_count: i64_to_u16(row.get(5)?)?,
            },
            border_node: if border == NONE_REF {
                u32::MAX
            } else {
                i64_to_u32(border)?
            },
        });
    }
    Ok(())
}
