// SPDX-License-Identifier: GPL-2.0-only

//! The gridlock checker's state (`TRAFFIC-14`, save format 109): one `gridlock_state` row with
//! the last check's report and, packed little-endian, what the next check needs: the stamps of
//! the lanes whose head waited at the last check and that check's cycles. The per-lane arrays
//! are not saved; a load sizes them for the loaded lanes.
//!
//! Save: O(lanes) for the head stamps plus O(heads + cycle lanes + cycle cars) bytes. Load:
//! O(heads + cycle lanes + cycle cars), one lane-remap lookup per saved lane.

use super::{
    SaveLoadError, SaveLoadResult, SnapshotMaps, i64_to_u32, i64_to_u64, optional_node_to_db,
    u64_to_i64,
};
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::agents::tick::{GridlockChecker, GridlockReport, SavedGridlock};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use rusqlite::{Connection, Transaction, params};
use std::collections::HashMap;

// Packs fixed-size records into one blob.
fn pack<T, const N: usize>(items: &[T], record: impl Fn(&T) -> [u8; N]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(items.len() * N);
    for item in items {
        blob.extend_from_slice(&record(item));
    }
    blob
}

// Unpacks a blob of fixed-size records; a partial record fails the load.
fn unpack<T, const N: usize>(
    blob: &[u8],
    record: impl Fn(&[u8; N]) -> T,
) -> SaveLoadResult<Vec<T>> {
    let (records, rest) = blob.as_chunks::<N>();
    if !rest.is_empty() {
        return Err(SaveLoadError::custom("gridlock blob size mismatch"));
    }
    Ok(records.iter().map(record).collect())
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("four bytes"))
}

fn le_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().expect("eight bytes"))
}

fn concat<const N: usize>(parts: &[&[u8]]) -> [u8; N] {
    let mut out = [0; N];
    let mut at = 0;
    for part in parts {
        out[at..at + part.len()].copy_from_slice(part);
        at += part.len();
    }
    out
}

/// Writes the checker's state for the lanes the save writes; its junctions in saved node ids.
pub(super) fn save(
    tx: &Transaction,
    agents: &AgentSystem,
    graph: &RegionGraph,
    network: &TransitNetwork,
    maps: &SnapshotMaps,
) -> SaveLoadResult<()> {
    let mut saved = agents.gridlock.save(network);
    let junctions = &mut saved.report.junctions;
    for node in junctions.iter_mut() {
        *node = i64_to_u32(optional_node_to_db(graph, *node, maps)?)?;
    }
    junctions.sort_unstable();
    junctions.dedup();
    write(tx, &saved)
}

// Writes the row for `saved` as given.
pub(super) fn write(tx: &Transaction, saved: &SavedGridlock) -> SaveLoadResult<()> {
    let report = &saved.report;
    tx.execute(
        "INSERT INTO gridlock_state(singleton, checks, waiting_lanes, cycles, confirmed, saturated, detours, saturation_reports, junctions, heads, cycle_lengths, cycle_lanes, cycle_cars) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            u64_to_i64(report.checks)?,
            i64::from(report.waiting_lanes),
            i64::from(report.cycles),
            i64::from(report.confirmed),
            i64::from(report.saturated),
            u64_to_i64(report.detours)?,
            u64_to_i64(report.saturation_reports)?,
            pack(&report.junctions, |node: &u32| node.to_le_bytes()),
            pack(&saved.heads, |&(lane, head, since): &(u32, u64, u64)| {
                concat::<20>(&[&lane.to_le_bytes(), &head.to_le_bytes(), &since.to_le_bytes()])
            }),
            pack(&saved.cycle_lengths, |length: &u32| length.to_le_bytes()),
            pack(&saved.cycle_lanes, |&(lane, cars): &(u32, u32)| {
                concat::<8>(&[&lane.to_le_bytes(), &cars.to_le_bytes()])
            }),
            pack(&saved.cycle_cars, |&(car, d): &(u64, u32)| {
                concat::<12>(&[&car.to_le_bytes(), &d.to_le_bytes()])
            }),
        ],
    )?;
    Ok(())
}

// Reads the row as saved, lane ids unmapped.
pub(super) fn read(conn: &Connection) -> SaveLoadResult<SavedGridlock> {
    let (counts, blobs): ([i64; 7], [Vec<u8>; 5]) = conn.query_row(
        "SELECT checks, waiting_lanes, cycles, confirmed, saturated, detours, saturation_reports, junctions, heads, cycle_lengths, cycle_lanes, cycle_cars FROM gridlock_state WHERE singleton = 1",
        [],
        |r| {
            Ok((
                [r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?],
                [r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?],
            ))
        },
    )?;
    let [
        checks,
        waiting_lanes,
        cycles,
        confirmed,
        saturated,
        detours,
        saturation_reports,
    ] = counts;
    let [junctions, heads, cycle_lengths, cycle_lanes, cycle_cars] = blobs;
    Ok(SavedGridlock {
        report: GridlockReport {
            checks: i64_to_u64(checks)?,
            waiting_lanes: i64_to_u32(waiting_lanes)?,
            cycles: i64_to_u32(cycles)?,
            confirmed: i64_to_u32(confirmed)?,
            saturated: i64_to_u32(saturated)?,
            detours: i64_to_u64(detours)?,
            saturation_reports: i64_to_u64(saturation_reports)?,
            junctions: unpack(&junctions, |b: &[u8; 4]| u32::from_le_bytes(*b))?,
        },
        heads: unpack(&heads, |b: &[u8; 20]| {
            (le_u32(&b[0..4]), le_u64(&b[4..12]), le_u64(&b[12..20]))
        })?,
        cycle_lengths: unpack(&cycle_lengths, |b: &[u8; 4]| u32::from_le_bytes(*b))?,
        cycle_lanes: unpack(&cycle_lanes, |b: &[u8; 8]| {
            (le_u32(&b[0..4]), le_u32(&b[4..8]))
        })?,
        cycle_cars: unpack(&cycle_cars, |b: &[u8; 12]| {
            (le_u64(&b[0..8]), le_u32(&b[8..12]))
        })?,
    })
}

/// Restores the checker into `agents`, its lanes mapped through `lane_remap` (saved lane id to
/// loaded lane id, from the lane references' restore).
pub(super) fn load(
    conn: &Connection,
    agents: &mut AgentSystem,
    lane_remap: &HashMap<usize, usize>,
) -> SaveLoadResult<()> {
    let mut saved = read(conn)?;
    saved
        .remap_lanes(|lane| lane_remap.get(&(lane as usize)).map(|&id| id as u32))
        .map_err(SaveLoadError::custom)?;
    agents.gridlock = GridlockChecker::restore(saved).map_err(SaveLoadError::custom)?;
    Ok(())
}

/// Writes `saved` as the only row of a save file at `path` and reads it back, for the ring
/// fixture's reload test: the file's table and encoding without a whole city.
#[cfg(test)]
pub(crate) fn round_trip_through_file(
    path: &std::path::Path,
    saved: &SavedGridlock,
) -> SaveLoadResult<SavedGridlock> {
    super::sqlite::write_sqlite_snapshot(path, super::schema::SCHEMA, |tx| write(tx, saved))?;
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    read(&conn)
}
