// SPDX-License-Identifier: GPL-2.0-only

//! Packed cell-zoning gestures; Godot owns pointer input while Rust owns selection and edits.

use super::*;
use crate::nodes::sim::CellToolPreview;
use crate::simulation::zoning::cells::{CellKey, CellSelection, CellSelectionShape, CellStore};
use glam::DVec2;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

/// Bounded bridge scheduling state; requests contain addresses, never captured world geometry.
#[derive(Default)]
pub(super) struct CellChunkJob {
    completion: Option<Receiver<()>>,
}

impl CellChunkJob {
    fn request(&mut self, commands: &Sender<SimCommand>, chunk: (i32, i32), world_generation: u64) {
        if self
            .completion
            .as_ref()
            .is_some_and(|completion| matches!(completion.try_recv(), Err(TryRecvError::Empty)))
        {
            return;
        }
        self.completion = None;
        let (completion, receiver) = mpsc::sync_channel(1);
        if commands
            .send(SimCommand::PrepareCellChunk {
                chunk,
                world_generation,
                completion,
            })
            .is_ok()
        {
            self.completion = Some(receiver);
        }
    }
}

#[godot_api(secondary)]
impl SimulationNode {
    /// Shared spatial chunk span used by the visible grid overlay; origins are world-zero aligned.
    #[func]
    pub fn get_zoning_cell_chunk_span() -> f64 {
        f64::from(crate::simulation::network::graph::RegionGraph::CHUNK_SIZE)
    }

    /// Polls visible chunk metadata in one nonblocking call. Each input XZ pair has five output
    /// values: cell revision, generation-complete flag, parcel revision and two height epochs.
    #[func]
    pub fn try_get_zoning_cell_chunk_states(&self, chunks: PackedInt32Array) -> VarDictionary {
        let mut result = VarDictionary::new();
        result.set("busy", true);
        let Some(core) = self.try_lock_core() else {
            return result;
        };
        if !chunks.len().is_multiple_of(2) {
            return result;
        }
        let mut states = PackedInt64Array::new();
        for chunk in chunks.as_slice().chunks_exact(2) {
            let (revision, generated) = core.zoning.cells.chunk_state((chunk[0], chunk[1]));
            states.extend([
                revision as i64,
                i64::from(generated),
                core.zoning.parcels.chunk_revision((chunk[0], chunk[1])) as i64,
                core.heightmap.source_generation() as i64,
                core.heightmap.visual_generation() as i64,
            ]);
        }
        result.set("busy", false);
        result.set("states", states);
        result
    }

    /// Exports a ready visible chunk without waiting on a busy simulation. Cold/dirty inputs
    /// queue bounded preparation on the simulation thread and return busy for a later poll.
    /// Unchanged chunks return only a version; centre ownership prevents duplicate uploads.
    #[func]
    pub fn try_get_zoning_cell_chunk_packed(
        &mut self,
        chunk_x: i32,
        chunk_z: i32,
        known: PackedInt64Array,
    ) -> VarDictionary {
        let mut result = VarDictionary::new();
        result.set("busy", true);
        let Some(core) = self.try_lock_core() else {
            return result;
        };
        let chunk = (chunk_x, chunk_z);
        if !core.cell_chunk_in_world(chunk) {
            result.set("busy", false);
            result.set("outside", true);
            return result;
        }
        if !core.zoning.cells.chunk_state(chunk).1
            || !core.allocator.building_site_query_index_ready()
            || !core
                .transit_network
                .road_surface
                .published_generation_matches_source()
        {
            let world_generation = core.terrain_payload_global_generation;
            // Release the core borrow before mutating bridge-only scheduling state. The worker
            // rechecks the reset epoch under its lock and derives from the latest local edits.
            drop(core);
            if self.sim_thread.is_some() {
                self.cell_chunk_job
                    .request(&self.cmd_tx, chunk, world_generation);
            }
            return result;
        }
        result.set("busy", false);
        let version = [
            core.zoning.cells.chunk_state(chunk).0 as i64,
            core.zoning.parcels.chunk_revision(chunk) as i64,
            core.heightmap.source_generation() as i64,
            core.heightmap.visual_generation() as i64,
        ];
        result.set("version", PackedInt64Array::from(version.as_slice()));
        let unchanged = known.as_slice() == version;
        result.set("unchanged", unchanged);
        if unchanged {
            return result;
        }
        let mut vertices = PackedVector3Array::new();
        let mut colors = PackedColorArray::new();
        let mut lines = PackedVector3Array::new();
        let mut line_colors = PackedColorArray::new();
        let mut keys = Vec::new();
        core.zoning
            .cells
            .visit_chunk_cells(chunk, |key| keys.push(key));
        keys.sort_unstable();
        for key in keys {
            let frame = core
                .zoning
                .cells
                .frame(key.grid)
                .expect("indexed cell frame");
            let corners = frame.corners(key.x, key.y);
            let centre = (corners[0] + corners[2]) * 0.5;
            let color =
                zoning_parcel_color(&core, core.zoning.cells.profile(key).unwrap_or(0), false);
            let corners = corners.map(|p| {
                // A visual inset leaves visible grid lines without altering authority or picking.
                let p = centre + (p - centre) * 0.94;
                let point = Vector2::new(p.x as f32, p.y as f32);
                Vector3::new(
                    point.x,
                    core.get_world_surface_height_internal(point) + 0.04,
                    point.y,
                )
            });
            for index in [0, 1, 2, 0, 2, 3] {
                vertices.push(corners[index]);
                colors.push(color);
            }
        }
        let bounds = CellStore::chunk_bounds(chunk);
        let span = Self::get_zoning_cell_chunk_span();
        core.zoning.parcels.visit_in_bounds(
            Vector2::new(bounds.min.x as f32, bounds.min.y as f32),
            Vector2::new(bounds.max.x as f32, bounds.max.y as f32),
            |parcel| {
                let center = parcel.center();
                if parcel.cell_lot().is_some()
                    || (
                        (f64::from(center.x) / span).floor() as i32,
                        (f64::from(center.y) / span).floor() as i32,
                    ) != chunk
                {
                    return;
                }
                let color = zoning_parcel_color(
                    &core,
                    parcel.zone_profile_runtime_id(),
                    parcel.occupied_building().is_some(),
                );
                let corners = parcel.corners().map(|point| {
                    Vector3::new(
                        point.x,
                        core.get_world_surface_height_internal(point) + 0.04,
                        point.y,
                    )
                });
                for index in [0, 1, 2, 0, 2, 3] {
                    vertices.push(corners[index]);
                    colors.push(color);
                }
                for index in 0..4 {
                    lines.extend([corners[index], corners[(index + 1) % 4]]);
                    line_colors.extend([Color::from_rgba(color.r, color.g, color.b, 0.9); 2]);
                }
            },
        );
        result.set("vertices", vertices);
        result.set("colors", colors);
        result.set("lines", lines);
        result.set("line_colors", line_colors);
        result
    }

    /// Returns the dependency tuple used to retain a cell preview while the cursor is idle.
    #[func]
    pub fn get_zoning_cell_dependencies(&self) -> PackedInt64Array {
        self.lock_core()
            .cell_tool_dependencies()
            .into_iter()
            .map(|v| v as i64)
            .collect()
    }

    /// Selects exact cells for Cell=0, Marquee=1, Fill=2 or Brush=3. The returned addresses,
    /// dependency tuple and packed corners describe the same transaction used by commit.
    #[func]
    pub fn get_zoning_cells_preview_packed(
        &self,
        shape: i32,
        path: PackedVector2Array,
        radius_m: f64,
        profile: i32,
    ) -> VarDictionary {
        let mut result = VarDictionary::new();
        result.set("valid", false);
        let Some(shape) = selection_shape(shape, radius_m) else {
            return result;
        };
        let Ok(profile) = u16::try_from(profile) else {
            return result;
        };
        let path: Vec<_> = path
            .as_slice()
            .iter()
            .map(|p| DVec2::new(f64::from(p.x), f64::from(p.y)))
            .collect();
        let mut core = self.lock_core();
        if profile != 0
            && core
                .zoning
                .profiles
                .profile_by_runtime_id(profile)
                .is_none()
        {
            return result;
        }
        let preview = core.preview_cell_selection_internal(shape, &path);
        let mut corners = PackedVector3Array::new();
        let mut addresses = PackedInt64Array::new();
        for &key in &preview.selection.cells {
            let frame = core
                .zoning
                .cells
                .frame(key.grid)
                .expect("selected cell frame");
            for corner in frame.corners(key.x, key.y) {
                let point = Vector2::new(corner.x as f32, corner.y as f32);
                corners.push(Vector3::new(
                    point.x,
                    core.get_world_surface_height_internal(point) + 0.08,
                    point.y,
                ));
            }
            addresses.extend([key.grid as i64, i64::from(key.x), i64::from(key.y)]);
        }
        let mut color = zoning_parcel_color(&core, profile, false);
        color.a = 0.65;
        let colors =
            PackedColorArray::from_iter(std::iter::repeat_n(color, preview.selection.cells.len()));
        result.set("valid", !preview.selection.cells.is_empty());
        result.set("cell_count", preview.selection.cells.len() as i64);
        result.set("cells", addresses);
        result.set(
            "dependencies",
            PackedInt64Array::from_iter(preview.dependencies.into_iter().map(|v| v as i64)),
        );
        result.set("corners", corners);
        result.set("colors", colors);
        result
    }

    /// Applies exactly the retained preview, rejecting stale dependencies and malformed addresses.
    /// Profile zero is Erase and uses the same selection and one-gesture undo transaction.
    #[func]
    pub fn apply_zoning_cells_preview(
        &mut self,
        cells: PackedInt64Array,
        dependencies: PackedInt64Array,
        profile: i32,
    ) -> bool {
        let Ok(profile) = u16::try_from(profile) else {
            return false;
        };
        let Some(preview) = decode_preview(cells.as_slice(), dependencies.as_slice()) else {
            return false;
        };
        self.lock_core()
            .apply_cell_selection_internal(&preview, profile)
    }
}

fn selection_shape(shape: i32, radius_m: f64) -> Option<CellSelectionShape> {
    match shape {
        0 => Some(CellSelectionShape::Cell),
        1 => Some(CellSelectionShape::Marquee),
        2 => Some(CellSelectionShape::Fill),
        3 if radius_m.is_finite() && radius_m >= 0.0 => {
            Some(CellSelectionShape::Brush { radius_m })
        }
        _ => None,
    }
}

fn decode_preview(addresses: &[i64], dependencies: &[i64]) -> Option<CellToolPreview> {
    if dependencies.len() != 11 || !addresses.len().is_multiple_of(3) {
        return None;
    }
    let mut cells = Vec::with_capacity(addresses.len() / 3);
    for address in addresses.chunks_exact(3) {
        let grid = u64::try_from(address[0]).ok().filter(|&grid| grid != 0)?;
        cells.push(CellKey {
            grid,
            x: i32::try_from(address[1]).ok()?,
            y: i32::try_from(address[2]).ok()?,
        });
    }
    cells.sort_unstable();
    if cells.windows(2).any(|pair| pair[0] == pair[1]) {
        return None;
    }
    let dependencies = std::array::from_fn(|i| dependencies[i] as u64);
    Some(CellToolPreview {
        selection: CellSelection {
            cells,
            revision: dependencies[8],
        },
        dependencies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_requests_are_bounded_and_retry_after_completion_or_disconnect() {
        let (commands, receiver) = mpsc::channel();
        let mut job = CellChunkJob::default();
        job.request(&commands, (0, -1), 7);
        let SimCommand::PrepareCellChunk {
            chunk,
            world_generation,
            completion,
        } = receiver.try_recv().unwrap()
        else {
            panic!("expected cell preparation")
        };
        assert_eq!((chunk, world_generation), ((0, -1), 7));
        for x in 0..100 {
            job.request(&commands, (x, 2), 8);
        }
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        completion.try_send(()).unwrap();
        job.request(&commands, (5, 6), 9);
        let SimCommand::PrepareCellChunk {
            chunk,
            world_generation,
            completion,
        } = receiver.try_recv().unwrap()
        else {
            panic!("expected cell preparation")
        };
        assert_eq!((chunk, world_generation), ((5, 6), 9));
        drop(completion);
        job.request(&commands, (-3, 4), 10);
        assert!(matches!(
            receiver.try_recv(),
            Ok(SimCommand::PrepareCellChunk {
                chunk: (-3, 4),
                world_generation: 10,
                ..
            })
        ));
        drop(receiver);
        job.request(&commands, (1, 1), 11);
        assert!(job.completion.is_none());
    }

    #[test]
    fn packed_gesture_rejects_truncated_duplicate_and_out_of_range_addresses() {
        let deps = [0; 11];
        for cells in [
            &[1, 2][..],
            &[0, 2, 3],
            &[-1, 2, 3],
            &[1, i64::MAX, 3],
            &[1, 2, 3, 1, 2, 3],
        ] {
            assert!(decode_preview(cells, &deps).is_none());
        }
        assert!(decode_preview(&[1, 2, 3], &deps[..10]).is_none());
        let mut deps = [0; 11];
        deps[8] = -1;
        let preview = decode_preview(
            &[2, -3, 4, 1, i64::from(i32::MIN), i64::from(i32::MAX)],
            &deps,
        )
        .unwrap();
        assert_eq!(preview.selection.revision, u64::MAX);
        assert_eq!(
            preview.selection.cells[0],
            CellKey {
                grid: 1,
                x: i32::MIN,
                y: i32::MAX
            }
        );
    }

    #[test]
    fn selector_codes_and_brush_validation_are_shared_by_paint_and_erase() {
        assert_eq!(selection_shape(0, 0.0), Some(CellSelectionShape::Cell));
        assert_eq!(selection_shape(1, 0.0), Some(CellSelectionShape::Marquee));
        assert_eq!(selection_shape(2, 0.0), Some(CellSelectionShape::Fill));
        assert_eq!(
            selection_shape(3, 20.0),
            Some(CellSelectionShape::Brush { radius_m: 20.0 })
        );
        for (shape, radius) in [(4, 10.0), (3, -1.0), (3, f64::NAN), (3, f64::INFINITY)] {
            assert!(selection_shape(shape, radius).is_none());
        }
    }
}
