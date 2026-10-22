// SPDX-License-Identifier: GPL-2.0-only

//! Cell zoning authority persistence; generated meshes, empty cells and indices are rebuilt.
//! The pending dirty lot regions are saved in service order, with the asset sizes the lots were
//! derived with (`SIM-02-H`).

use super::{
    SaveLoadError, SaveLoadResult, SnapshotMaps, i64_to_u16, i64_to_u64, i64_to_usize, u64_to_i64,
    usize_to_i64,
};
use crate::simulation::agriculture::{AgricultureSystem, PolygonFootprint};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::RoadSurfaceSystem;
use crate::simulation::zoning::cells::{
    CellCurveSource, CellKey, CellLot, GridFrame, RoadCellAlignment,
};
use crate::simulation::zoning::{ZoneType, ZoningSystem};
use rayon::prelude::*;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::{BTreeMap, HashSet};

/// Writes authoritative frames, reservations and validated lot coverage in one save transaction.
pub(super) fn save(
    tx: &Transaction<'_>,
    zoning: &ZoningSystem,
    maps: &SnapshotMaps,
) -> SaveLoadResult<()> {
    let mut alignment_stmt =
        tx.prepare("INSERT INTO zoning_cell_road_alignments VALUES (?1, ?2)")?;
    for (id, alignment) in zoning.cells.saved_road_alignments() {
        if let Some(&saved_id) = maps.edge_old_to_new.get(&id) {
            let json = serde_json::to_string(&alignment)
                .map_err(|err| SaveLoadError::custom(err.to_string()))?;
            alignment_stmt.execute(params![usize_to_i64(saved_id)?, json])?;
        }
    }
    let mut frame_stmt = tx.prepare("INSERT INTO zoning_cell_frames VALUES (?1, ?2)")?;
    for (id, frame) in zoning.cells.saved_frames() {
        let json =
            serde_json::to_string(&frame).map_err(|err| SaveLoadError::custom(err.to_string()))?;
        frame_stmt.execute(params![u64_to_i64(id)?, json])?;
    }
    let mut cell_stmt = tx.prepare("INSERT INTO zoning_cells VALUES (?1, ?2, ?3, ?4)")?;
    for (key, profile) in zoning.cells.saved_cells() {
        cell_stmt.execute(params![
            u64_to_i64(key.grid)?,
            key.x,
            key.y,
            i64::from(profile)
        ])?;
    }
    let mut source_stmt = tx.prepare("INSERT INTO zoning_cell_curve_sources VALUES (?1, ?2)")?;
    for (index, source) in zoning.cells.saved_curve_sources().into_iter().enumerate() {
        let json =
            serde_json::to_string(source).map_err(|err| SaveLoadError::custom(err.to_string()))?;
        source_stmt.execute(params![u64_to_i64(index as u64)?, json])?;
    }
    let mut lot_stmt = tx.prepare("INSERT INTO zoning_cell_lots VALUES (?1, ?2)")?;
    for parcel in zoning.parcels() {
        if let Some(lot) = parcel.cell_lot() {
            if !lot
                .cells()
                .all(|key| zoning.cells.lot(key) == Some(parcel.id().raw()))
            {
                return Err(SaveLoadError::custom(
                    "cell lot coverage does not match parcel claim",
                ));
            }
            let json = serde_json::to_string(&lot)
                .map_err(|err| SaveLoadError::custom(err.to_string()))?;
            lot_stmt.execute(params![u64_to_i64(parcel.id().raw())?, json])?;
        }
    }
    let mut queue_stmt = tx.prepare("INSERT INTO zoning_cell_lot_queue VALUES (?1, ?2, ?3, ?4)")?;
    for (position, ((x, y), queued)) in zoning.saved_cell_lot_queue().into_iter().enumerate() {
        queue_stmt.execute(params![usize_to_i64(position)?, x, y, i64::from(queued)])?;
    }
    if let Some(sizes) = zoning.saved_cell_lot_sizes() {
        let json =
            serde_json::to_string(sizes).map_err(|err| SaveLoadError::custom(err.to_string()))?;
        tx.execute(
            "INSERT INTO zoning_cell_lot_catalog VALUES (1, ?1)",
            params![json],
        )?;
    }
    Ok(())
}

/// Restores the pending dirty lot regions in saved order, replacing the regions the parcel
/// restore marked: the saved lots already hold their derivation, so a loaded game serves the
/// same regions in the same slots as the game that saved it.
pub(super) fn load_lot_queue(conn: &Connection, zoning: &mut ZoningSystem) -> SaveLoadResult<()> {
    let mut stmt = conn
        .prepare("SELECT chunk_x, chunk_y, queued FROM zoning_cell_lot_queue ORDER BY position")?;
    let mut rows = stmt.query([])?;
    let mut entries: Vec<((i32, i32), bool)> = Vec::new();
    while let Some(row) = rows.next()? {
        let queued: i64 = row.get(2)?;
        if !matches!(queued, 0 | 1) {
            return Err(SaveLoadError::custom("invalid saved cell lot queue entry"));
        }
        entries.push(((row.get(0)?, row.get(1)?), queued == 1));
    }
    if !zoning.restore_cell_lot_queue(entries) {
        return Err(SaveLoadError::custom(
            "duplicate saved cell lot queue chunk",
        ));
    }
    // The sizes the saved lots were derived with, checked against the live catalog when the
    // loaded world is applied (`ZoningSystem::restore_cell_lot_assets`).
    let json: Option<String> = conn
        .query_row(
            "SELECT sizes_json FROM zoning_cell_lot_catalog WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    zoning.cell_lot_sizes = json
        .map(|json| serde_json::from_str(&json))
        .transpose()
        .map_err(|err| SaveLoadError::custom(format!("invalid saved cell lot sizes: {err}")))?;
    Ok(())
}

/// Validates saved frames/paint and returns coverage for the subsequent parcel restoration pass.
pub(super) fn load(
    conn: &Connection,
    zoning: &mut ZoningSystem,
    graph: &RegionGraph,
) -> SaveLoadResult<BTreeMap<u64, CellLot>> {
    let mut stmt = conn.prepare(
        "SELECT edge_id, alignment_json FROM zoning_cell_road_alignments ORDER BY edge_id",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let id = i64_to_usize(row.get(0)?)?;
        let json: String = row.get(1)?;
        let alignment: RoadCellAlignment = serde_json::from_str(&json)
            .map_err(|err| SaveLoadError::custom(format!("invalid road grid alignment: {err}")))?;
        if !zoning
            .cells
            .restore_road_alignment(id, alignment, graph, &zoning.config)
        {
            return Err(SaveLoadError::custom(
                "invalid or duplicate road grid alignment",
            ));
        }
    }
    let mut stmt =
        conn.prepare("SELECT grid_id, frame_json FROM zoning_cell_frames ORDER BY grid_id")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let grid = i64_to_u64(row.get(0)?)?;
        let json: String = row.get(1)?;
        let frame: GridFrame = serde_json::from_str(&json)
            .map_err(|err| SaveLoadError::custom(format!("invalid cell frame: {err}")))?;
        if GridFrame::new(
            glam::DVec2::ZERO,
            glam::DVec2::X,
            f64::from(zoning.config.zone_cell_m),
        )
        .is_none_or(|expected| frame.cell_m() != expected.cell_m())
            || !zoning.cells.restore_saved_frame(grid, frame)
        {
            return Err(SaveLoadError::custom(
                "invalid or duplicate saved cell frame",
            ));
        }
    }
    let mut stmt = conn.prepare(
        "SELECT grid_id, x, y, profile_runtime_id FROM zoning_cells ORDER BY grid_id, x, y",
    )?;
    let mut rows = stmt.query([])?;
    let mut unpainted = HashSet::new();
    while let Some(row) = rows.next()? {
        let key = CellKey {
            grid: i64_to_u64(row.get(0)?)?,
            x: row.get(1)?,
            y: row.get(2)?,
        };
        let profile = i64_to_u16(row.get(3)?)?;
        let frame = zoning
            .cells
            .frame(key.grid)
            .ok_or_else(|| SaveLoadError::custom("saved cell references missing grid"))?;
        let inside = frame.corners(key.x, key.y).iter().all(|p| {
            p.x.abs() <= f64::from(zoning.config.width_m) * 0.5
                && p.y.abs() <= f64::from(zoning.config.height_m) * 0.5
        });
        if !inside
            || (profile != 0 && zoning.profiles.profile_by_runtime_id(profile).is_none())
            || !zoning.cells.restore_saved_cell(key, profile)
        {
            return Err(SaveLoadError::custom(
                "invalid, duplicate or overlapping saved zoning cell",
            ));
        }
        if profile == 0 {
            unpainted.insert(key);
        }
    }
    let mut lots = BTreeMap::new();
    let mut stmt =
        conn.prepare("SELECT parcel_id, coverage_json FROM zoning_cell_lots ORDER BY parcel_id")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let id = i64_to_u64(row.get(0)?)?;
        let json: String = row.get(1)?;
        let lot: CellLot = serde_json::from_str(&json)
            .map_err(|err| SaveLoadError::custom(format!("invalid cell lot coverage: {err}")))?;
        if id == 0 || !zoning.cells.can_restore_lot(lot) || lots.insert(id, lot).is_some() {
            return Err(SaveLoadError::custom("invalid saved cell lot coverage"));
        }
        for key in lot.cells() {
            unpainted.remove(&key);
        }
    }
    if !unpainted.is_empty() {
        return Err(SaveLoadError::custom(
            "saved unpainted cell has no lot claim",
        ));
    }
    let mut stmt =
        conn.prepare("SELECT source_json FROM zoning_cell_curve_sources ORDER BY source_id")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let json: String = row.get(0)?;
        let source: CellCurveSource = serde_json::from_str(&json)
            .map_err(|err| SaveLoadError::custom(format!("invalid cell curve source: {err}")))?;
        if !zoning.cells.restore_curve_source(source) {
            return Err(SaveLoadError::custom(
                "invalid or duplicate saved cell curve source",
            ));
        }
    }
    Ok(lots)
}

/// Checks cross-system cell reservations after saved occupancy, sites and roads are reconstructed.
pub(super) fn validate_reservations(
    zoning: &ZoningSystem,
    allocator: &BuildingAllocator,
    agriculture: &AgricultureSystem,
    roads: &RoadSurfaceSystem,
) -> SaveLoadResult<()> {
    if !zoning.cells.has_reservations() {
        return Ok(());
    }
    // This one-time load pass visits each external owner and uses existing cell chunk/mask
    // queries for its footprint. It never compares every saved cell with every external owner.
    if agriculture
        .sites()
        .par_iter()
        .any(|site| zoning.cells_overlap_site(&PolygonFootprint::new(&site.polygon_world)))
    {
        return Err(SaveLoadError::custom(
            "saved field overlaps reserved zoning cells",
        ));
    }
    if allocator
        .buildings
        .par_iter()
        .zip(&allocator.building_sites)
        .any(|(building, site)| {
            building.parcel_id == 0
                && building.zone_type == ZoneType::None
                && zoning.cells_overlap_site(&PolygonFootprint::new(&site.lot_footprint_world))
        })
    {
        return Err(SaveLoadError::custom(
            "saved explicit site overlaps reserved zoning cells",
        ));
    }
    let overlaps = |polygon| zoning.cells_overlap_road_polygon(polygon);
    if roads
        .compiled_visual_span_pieces
        .par_iter()
        .any(|(_, piece)| {
            piece
                .surface_polygons()
                .any(|quad| zoning.cells_overlap_road_points_world(quad.points()))
        })
        || roads
            .compiled_visual_node_pieces
            .par_iter()
            .any(|(_, piece)| piece.surface_polygons().any(overlaps))
    {
        return Err(SaveLoadError::custom(
            "saved road overlaps reserved zoning cells",
        ));
    }
    Ok(())
}
