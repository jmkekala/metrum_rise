// SPDX-License-Identifier: GPL-2.0-only

//! Scheduler state (`SIM-02`): the timing wheel and its counters, saved as generic rows so
//! this module does not depend on the simulation's event types.

use super::{SaveLoadError, SaveLoadResult, i64_to_u16, i64_to_u32, i64_to_u64, u64_to_i64};
use crate::simulation::core::scheduler::SLOTS_PER_DAY;
use crate::simulation::grid::sweep::{EnvironmentSweep, SweepCapacities};
use crate::simulation::network::graph::RegionGraph;
use rusqlite::{Connection, Transaction, params};

/// The timing wheel and its counters as saved.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SavedScheduler {
    /// First slot the wheel has not popped.
    pub(crate) next_slot: u64,
    /// Next event insertion sequence.
    pub(crate) next_event_seq: u64,
    /// Next queued demand spawn id.
    pub(crate) next_spawn_id: u64,
    /// Clock of building freight cooldowns (`ShipmentSystem::hour_ticks`).
    pub(crate) logistics_hour_ticks: u32,
    /// Clock of job locks (`AgentSystem::settlement_days`).
    pub(crate) agent_settlement_days: u32,
    /// Rolling road length for daily upkeep (`SIM-02-G`): last finished sweep in metres,
    /// current sweep in millimetres, edges frozen for it (saved as -1 before the first sweep)
    /// and the next edge, both in saved edge numbering, and whether a sweep has finished.
    pub(crate) road_length: (f64, i64, Option<u64>, u64, bool),
    /// Rolling environment sweep (`SIM-02-H`) with live edge indices; `save` stores them as
    /// indices into the saved edges, which leave out deleted ones.
    pub(crate) environment: EnvironmentSweep,
    /// `(slot, seq, kind, a)` per pending event, in firing order; `kind` and `a` are owned by
    /// the simulation's event type.
    pub(crate) events: Vec<(u64, u64, u16, u64)>,
}

pub(super) fn save(
    tx: &Transaction,
    scheduler: &SavedScheduler,
    graph: &RegionGraph,
) -> SaveLoadResult<()> {
    let environment = scheduler.environment;
    let saved_edge =
        |index| compacted_edge_index(graph.edges().iter().map(|edge| edge.deleted), index);
    let (environment_handles, environment_edges) = match environment.frozen {
        Some(frozen) => (
            u64_to_i64(frozen.building_handles)?,
            u64_to_i64(saved_edge(frozen.edges))?,
        ),
        None => (-1, 0),
    };
    tx.execute(
        "INSERT INTO scheduler_state(singleton, next_slot, next_event_seq, next_spawn_id, logistics_hour_ticks, agent_settlement_days, road_length_front_m, road_length_back_mm, road_length_edges, road_length_next_edge, road_length_measured, environment_building_handles, environment_edges, environment_next_edge, environment_next_chunk) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            u64_to_i64(scheduler.next_slot)?,
            u64_to_i64(scheduler.next_event_seq)?,
            u64_to_i64(scheduler.next_spawn_id)?,
            i64::from(scheduler.logistics_hour_ticks),
            i64::from(scheduler.agent_settlement_days),
            scheduler.road_length.0,
            scheduler.road_length.1,
            match scheduler.road_length.2 {
                Some(edges) => u64_to_i64(edges)?,
                None => -1,
            },
            u64_to_i64(scheduler.road_length.3)?,
            i64::from(scheduler.road_length.4),
            environment_handles,
            environment_edges,
            u64_to_i64(saved_edge(environment.next_edge))?,
            u64_to_i64(environment.next_chunk)?,
        ],
    )?;
    let mut events =
        tx.prepare("INSERT INTO timed_events(slot, seq, kind, a) VALUES (?1, ?2, ?3, ?4)")?;
    for &(slot, seq, kind, a) in &scheduler.events {
        events.execute(params![
            u64_to_i64(slot)?,
            u64_to_i64(seq)?,
            i64::from(kind),
            u64_to_i64(a)?,
        ])?;
    }
    Ok(())
}

// front_m, back_mm, frozen edges (-1 before a sweep), next edge, measured.
type SavedRoadLengthRow = (f64, i64, i64, i64, i64);

pub(super) fn load(conn: &Connection) -> SaveLoadResult<SavedScheduler> {
    // Clock and event counters, then the road-length sweep columns.
    let row: (i64, i64, i64, i64, i64, SavedRoadLengthRow) = conn.query_row(
        "SELECT next_slot, next_event_seq, next_spawn_id, logistics_hour_ticks, agent_settlement_days, road_length_front_m, road_length_back_mm, road_length_edges, road_length_next_edge, road_length_measured FROM scheduler_state WHERE singleton = 1",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                (
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                ),
            ))
        },
    )?;
    let mut scheduler = SavedScheduler {
        next_slot: i64_to_u64(row.0)?,
        next_event_seq: i64_to_u64(row.1)?,
        next_spawn_id: i64_to_u64(row.2)?,
        logistics_hour_ticks: i64_to_u32(row.3)?,
        agent_settlement_days: i64_to_u32(row.4)?,
        road_length: (
            row.5.0,
            row.5.1,
            if row.5.2 == -1 {
                None
            } else {
                Some(i64_to_u64(row.5.2)?)
            },
            i64_to_u64(row.5.3)?,
            row.5.4 != 0,
        ),
        environment: load_environment(conn)?,
        events: Vec::new(),
    };
    let mut stmt = conn.prepare("SELECT slot, seq, kind, a FROM timed_events ORDER BY slot, seq")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        scheduler.events.push((
            i64_to_u64(row.get(0)?)?,
            i64_to_u64(row.get(1)?)?,
            i64_to_u16(row.get(2)?)?,
            i64_to_u64(row.get(3)?)?,
        ));
    }
    if scheduler
        .events
        .iter()
        .any(|&(slot, seq, _, _)| slot < scheduler.next_slot || seq >= scheduler.next_event_seq)
    {
        return Err(SaveLoadError::custom(
            "timed event outside the saved scheduler window",
        ));
    }
    Ok(scheduler)
}

// Index of live edge `index` among the saved edges, which leave out deleted ones; an index past
// the live edges shifts by the same amount, so edges added after loading keep their order.
pub(super) fn compacted_edge_index(deleted: impl Iterator<Item = bool>, index: u64) -> u64 {
    let deleted_below = deleted.take(index as usize).filter(|&flag| flag).count();
    index - deleted_below as u64
}

fn load_environment(conn: &Connection) -> SaveLoadResult<EnvironmentSweep> {
    let (handles, edges, next_edge, next_chunk): (i64, i64, i64, i64) = conn.query_row(
        "SELECT environment_building_handles, environment_edges, environment_next_edge, environment_next_chunk FROM scheduler_state WHERE singleton = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let sweep = EnvironmentSweep {
        frozen: match handles {
            -1 => None,
            _ => Some(SweepCapacities {
                building_handles: i64_to_u64(handles)?,
                edges: i64_to_u64(edges)?,
            }),
        },
        next_chunk: i64_to_u64(next_chunk)?,
        next_edge: i64_to_u64(next_edge)?,
    };
    if sweep.next_chunk > SLOTS_PER_DAY {
        return Err(SaveLoadError::custom(
            "environment sweep chunk outside the day",
        ));
    }
    Ok(sweep)
}
