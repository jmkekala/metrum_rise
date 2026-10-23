// SPDX-License-Identifier: GPL-2.0-only

//! The road chunk index: render chunks and fine query cells covered by each published piece.
//!
//! Render chunks follow the terrain-aligned grid; a piece covers every chunk its triangles'
//! padded bounds touch, a superset of its triangles' centroid home chunks. Query cells are a
//! fixed 32 m grid, the legacy query-chunk span, so a point query sees only the pieces near it
//! however large the render chunks are. Both are persistent maps, so snapshot clones are `O(1)`.

use super::SurfaceChunkKey;
use super::mesh::PieceMesh;
use super::query::PieceId;
use std::sync::Arc;

/// Fine query-cell key `(x, z)` in the fixed [`QUERY_CELL_M`] grid from the world origin.
pub(super) type QueryCell = (i32, i32);

/// Query-cell span in metres.
pub(super) const QUERY_CELL_M: f64 = 32.0;
// Bounds padding: covers the f32 rounding of rendered vertices (2 mm at 20 km).
const FOOTPRINT_PAD_M: f64 = 0.01;

/// Terrain-aligned render chunk grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RenderGrid {
    pub(super) span_m: f32,
    pub(super) origin_m: (f32, f32),
}

impl RenderGrid {
    pub(super) fn key_at(&self, x: f64, z: f64) -> SurfaceChunkKey {
        let span = f64::from(self.span_m);
        (
            ((x - f64::from(self.origin_m.0)) / span).floor() as i32,
            ((z - f64::from(self.origin_m.1)) / span).floor() as i32,
        )
    }
}

pub(super) fn query_cell_at(x: f64, z: f64) -> QueryCell {
    (
        (x / QUERY_CELL_M).floor() as i32,
        (z / QUERY_CELL_M).floor() as i32,
    )
}

/// Render chunks and query cells a piece covers, each sorted and unique.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct PieceFootprint {
    render_chunks: Box<[SurfaceChunkKey]>,
    query_cells: Box<[QueryCell]>,
}

impl PieceFootprint {
    /// Footprint of a span mesh from the bounds of each pair of consecutive section rows.
    /// `O(rows * stride + covered cells)`.
    pub(super) fn of_span(
        mesh: &PieceMesh,
        stride: usize,
        grid: RenderGrid,
        scratch: &mut Vec<(i32, i32)>,
    ) -> Self {
        let rows = mesh.positions().len() / stride.max(1);
        let pair_bounds = (1..rows).map(|row| {
            mesh.positions()[(row - 1) * stride..(row + 1) * stride]
                .iter()
                .fold(
                    [
                        f64::INFINITY,
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                        f64::NEG_INFINITY,
                    ],
                    |bounds, point| {
                        [
                            bounds[0].min(point.x),
                            bounds[1].min(point.z),
                            bounds[2].max(point.x),
                            bounds[3].max(point.z),
                        ]
                    },
                )
        });
        Self {
            render_chunks: covered_keys(pair_bounds.clone(), scratch, |x, z| grid.key_at(x, z)),
            query_cells: covered_keys(pair_bounds, scratch, query_cell_at),
        }
    }

    pub(super) fn render_chunks(&self) -> &[SurfaceChunkKey] {
        &self.render_chunks
    }
}

fn covered_keys(
    bounds: impl Iterator<Item = [f64; 4]>,
    scratch: &mut Vec<(i32, i32)>,
    key_at: impl Fn(f64, f64) -> (i32, i32),
) -> Box<[(i32, i32)]> {
    scratch.clear();
    for [min_x, min_z, max_x, max_z] in bounds {
        let first = key_at(min_x - FOOTPRINT_PAD_M, min_z - FOOTPRINT_PAD_M);
        let last = key_at(max_x + FOOTPRINT_PAD_M, max_z + FOOTPRINT_PAD_M);
        for x in first.0..=last.0 {
            for z in first.1..=last.1 {
                scratch.push((x, z));
            }
        }
    }
    scratch.sort_unstable();
    scratch.dedup();
    scratch.as_slice().into()
}

/// A published piece as the query cells list it.
#[derive(Clone)]
pub(super) struct IndexedPiece {
    pub(super) id: PieceId,
    pub(super) mesh: Arc<PieceMesh>,
}

/// Query cells and render-chunk reference counts of the published pieces.
#[derive(Clone, Default)]
pub(super) struct PieceIndex {
    // Sorted by piece id within each cell.
    cells: imbl::HashMap<QueryCell, Arc<Vec<IndexedPiece>>>,
    chunk_refs: imbl::OrdMap<SurfaceChunkKey, u32>,
    // Grows with every insert and resets on clear: bounds rays and region scans.
    extent_xz: Option<[f64; 4]>,
}

impl PieceIndex {
    pub(super) fn insert(
        &mut self,
        id: PieceId,
        mesh: &Arc<PieceMesh>,
        footprint: &PieceFootprint,
    ) {
        for &cell in footprint.query_cells.iter() {
            let entry = IndexedPiece {
                id,
                mesh: Arc::clone(mesh),
            };
            if let Some(list) = self.cells.get_mut(&cell) {
                let list = Arc::make_mut(list);
                let position = list.partition_point(|indexed| indexed.id < id);
                list.insert(position, entry);
            } else {
                self.cells.insert(cell, Arc::new(vec![entry]));
            }
        }
        for &chunk in footprint.render_chunks.iter() {
            *self.chunk_refs.entry(chunk).or_insert(0) += 1;
        }
        let bounds = mesh.bounds_xz();
        self.extent_xz = Some(self.extent_xz.map_or(bounds, |extent| {
            [
                extent[0].min(bounds[0]),
                extent[1].min(bounds[1]),
                extent[2].max(bounds[2]),
                extent[3].max(bounds[3]),
            ]
        }));
    }

    pub(super) fn remove(&mut self, id: PieceId, footprint: &PieceFootprint) {
        for &cell in footprint.query_cells.iter() {
            let Some(list) = self.cells.get_mut(&cell) else {
                continue;
            };
            let list = Arc::make_mut(list);
            list.retain(|indexed| indexed.id != id);
            if list.is_empty() {
                self.cells.remove(&cell);
            }
        }
        for &chunk in footprint.render_chunks.iter() {
            let Some(count) = self.chunk_refs.get_mut(&chunk) else {
                continue;
            };
            *count -= 1;
            if *count == 0 {
                self.chunk_refs.remove(&chunk);
            }
        }
    }

    pub(super) fn cell(&self, cell: QueryCell) -> &[IndexedPiece] {
        self.cells.get(&cell).map_or(&[], |list| list.as_slice())
    }

    pub(super) fn occupied_chunks(&self) -> impl Iterator<Item = SurfaceChunkKey> + '_ {
        self.chunk_refs.keys().copied()
    }

    pub(super) fn extent_xz(&self) -> Option<[f64; 4]> {
        self.extent_xz
    }
}
