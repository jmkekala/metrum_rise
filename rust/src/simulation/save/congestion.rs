// SPDX-License-Identifier: GPL-2.0-only

//! The car-congestion metric's state (`TRAFFIC-17`, save format 119): one `congestion_routing`
//! row with the adopted metric's directed weights and, while a customization is pending, its
//! snapshot weights and adoption slot. Weights are packed little-endian f32, two per edge
//! (forward, backward) in saved edge id order. A load customizes both again, pure functions of
//! the weights, so the loaded game routes cars and adopts at the same slot as the saved one.
//! The lanes' smoothed routing delays are saved with the lanes (`saved_lanes.route_delay_s`).
//!
//! Save: O(edges) bytes. Load: O(edges) plus two customizations.

use super::{SaveLoadError, SaveLoadResult, SnapshotMaps, i64_to_u64, u64_to_i64};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

// The saved row: adopted weights, pending weights and the pending adoption slot.
type SavedCongestion = (Option<Vec<u8>>, Option<Vec<u8>>, Option<i64>);

// Directed weights indexed by edge id, packed in edge id order (`SAVE-11`: ids are saved
// state); a deleted edge's pair is written as zero, as its hole loads.
fn pack(weights: &[f32], maps: &SnapshotMaps, edge_count: usize) -> SaveLoadResult<Vec<u8>> {
    if weights.len() != edge_count * 2 {
        return Err(SaveLoadError::custom(
            "congestion weights do not cover the edges",
        ));
    }
    Ok(weights
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
        .flat_map(|(edge, pair)| {
            let pair = if maps.edge(edge).is_some() { *pair } else { [0.0; 2] };
            pair.into_iter().flat_map(f32::to_le_bytes)
        })
        .collect())
}

fn unpack(blob: &[u8], graph: &RegionGraph) -> SaveLoadResult<Vec<f32>> {
    let (records, rest) = blob.as_chunks::<4>();
    let weights: Vec<f32> = records
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect();
    if !rest.is_empty()
        || weights.len() != graph.edge_count() * 2
        || weights
            .iter()
            .any(|weight| !weight.is_finite() || *weight < 0.0)
    {
        return Err(SaveLoadError::custom("invalid saved congestion weights"));
    }
    Ok(weights)
}

pub(super) fn save(
    tx: &Transaction,
    graph: &RegionGraph,
    network: &TransitNetwork,
    maps: &SnapshotMaps,
) -> SaveLoadResult<()> {
    let edges = graph.edge_count();
    let adopted = network
        .cch_graph
        .congestion_weights()
        .map(|weights| pack(weights, maps, edges))
        .transpose()?;
    let pending = network
        .pending_congestion()
        .map(|(weights, slot)| {
            Ok::<_, SaveLoadError>((pack(weights, maps, edges)?, u64_to_i64(slot)?))
        })
        .transpose()?;
    tx.execute(
        "INSERT INTO congestion_routing(singleton, adopted, pending, adopt_slot) VALUES (1, ?1, ?2, ?3)",
        params![adopted, pending.as_ref().map(|p| &p.0), pending.as_ref().map(|p| p.1)],
    )?;
    Ok(())
}

/// Restores the adopted and pending congestion metrics on the loaded hierarchy, which must be
/// built already.
pub(super) fn load(
    conn: &Connection,
    graph: &RegionGraph,
    network: &mut TransitNetwork,
) -> SaveLoadResult<()> {
    let row: Option<SavedCongestion> = conn
        .query_row(
            "SELECT adopted, pending, adopt_slot FROM congestion_routing WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((adopted, pending, slot)) = row else {
        return Err(SaveLoadError::custom("missing congestion routing state"));
    };
    if let Some(blob) = adopted {
        network.install_congestion(unpack(&blob, graph)?);
    }
    match (pending, slot) {
        (Some(blob), Some(slot)) => {
            network.restore_pending_congestion(unpack(&blob, graph)?, i64_to_u64(slot)?)
        }
        (None, None) => {}
        _ => return Err(SaveLoadError::custom("pending congestion without its slot")),
    }
    Ok(())
}
