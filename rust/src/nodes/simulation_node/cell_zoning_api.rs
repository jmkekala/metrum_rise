// SPDX-License-Identifier: GPL-2.0-only

//! Packed cell-zoning gestures; Godot owns pointer input while Rust owns selection and edits.

use super::*;
use crate::nodes::sim::CellToolPreview;
use crate::nodes::sim::render::zoning_mask::{MaskRectangle, build_mask};
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

    /// Reports cell overlay visibility without waiting on the simulation. While shown, chunks a
    /// local edit invalidates are regenerated before that edit's snapshot is published, so the
    /// first poll after a road appears already exports its cells. Call on visibility changes.
    #[func]
    pub fn set_zoning_cell_overlay_visible(&mut self, visible: bool) {
        let _ = self.cmd_tx.send(SimCommand::SetCellOverlayVisible(visible));
    }

    /// Polls visible chunk metadata in one nonblocking call. Each input XZ pair has four output
    /// values: cell revision (including crossing owners), generation-complete flag, parcel
    /// revision and world generation. Height alone never changes ground-mask geometry.
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
        for chunk in chunks.as_slice().as_chunks::<2>().0 {
            let version = mask_version(&core, (chunk[0], chunk[1]));
            let generated = core.zoning.cells.chunk_state((chunk[0], chunk[1])).1;
            states.extend([version[0], i64::from(generated), version[1], version[2]]);
        }
        result.set("busy", false);
        result.set("states", states);
        result
    }

    /// Exports a ready visible chunk without waiting on a busy simulation. Cold/dirty inputs
    /// queue bounded preparation on the simulation thread and return busy for a later poll.
    /// Unchanged chunks return only a version. Crossing rectangles occur in both masks,
    /// clipped by the shader to the fragment's chunk; no ground fragment is drawn twice.
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
            || !core.transit_network.roadbed.is_current()
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
        let version = mask_version(&core, chunk);
        result.set("version", PackedInt64Array::from(version.as_slice()));
        let unchanged = known.as_slice() == version;
        result.set("unchanged", unchanged);
        if unchanged {
            return result;
        }
        let bounds = CellStore::chunk_bounds(chunk);
        let mut keys = Vec::new();
        core.zoning
            .cells
            .visit_in_bounds(bounds, |key| keys.push(key));
        keys.sort_unstable();
        let mut rectangles = Vec::with_capacity(keys.len());
        for key in keys {
            let frame = core
                .zoning
                .cells
                .frame(key.grid)
                .expect("indexed cell frame");
            let profile = core.zoning.cells.profile(key).unwrap_or(0);
            let occupied = core
                .zoning
                .cells
                .lot(key)
                .and_then(|id| {
                    core.zoning
                        .parcels
                        .get(crate::simulation::zoning::ParcelId::from_raw(id))
                })
                .is_some_and(|parcel| parcel.occupied_building().is_some());
            let style = if occupied {
                0.0
            } else if profile == 0 {
                0.5
            } else {
                1.0
            };
            rectangles.push(MaskRectangle {
                corners: frame.corners(key.x, key.y),
                color_style: mask_color(zoning_parcel_color(&core, profile, false), style),
            });
        }
        // ParcelStore's query is indexed but not ordered. Stable IDs define render precedence.
        let mut parcels = Vec::new();
        core.zoning.parcels.visit_in_bounds(
            Vector2::new(bounds.min.x as f32, bounds.min.y as f32),
            Vector2::new(bounds.max.x as f32, bounds.max.y as f32),
            |parcel| {
                if parcel.cell_lot().is_none() {
                    let style = if parcel.occupied_building().is_some() {
                        0.0
                    } else if parcel.zone_profile_runtime_id() == 0 {
                        0.5
                    } else {
                        1.0
                    };
                    parcels.push((
                        parcel.id(),
                        MaskRectangle {
                            corners: parcel
                                .corners()
                                .map(|p| DVec2::new(f64::from(p.x), f64::from(p.y))),
                            color_style: mask_color(
                                zoning_parcel_color(&core, parcel.zone_profile_runtime_id(), false),
                                style,
                            ),
                        },
                    ));
                }
            },
        );
        parcels.sort_unstable_by_key(|(id, _)| *id);
        rectangles.extend(parcels.into_iter().map(|(_, rectangle)| rectangle));
        // Rasterization reads this immutable local snapshot, never holds the simulation lock.
        drop(core);
        export_mask(&mut result, chunk, &rectangles);
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
        let mut masks: std::collections::BTreeMap<(i32, i32), Vec<MaskRectangle>> =
            std::collections::BTreeMap::new();
        let preview_color = mask_color(zoning_parcel_color(&core, profile, false), 2.0);
        for &key in &preview.selection.cells {
            let frame = core
                .zoning
                .cells
                .frame(key.grid)
                .expect("selected cell frame");
            let points = frame.corners(key.x, key.y);
            for corner in points {
                corners.push(Vector3::new(corner.x as f32, 0.0, corner.y as f32));
            }
            let bounds = crate::simulation::zoning::cells::CellBounds::from_points(points);
            let min = (bounds.min / 512.0).floor();
            let max = (bounds.max / 512.0).floor();
            for z in min.y as i32..=max.y as i32 {
                for x in min.x as i32..=max.x as i32 {
                    masks.entry((x, z)).or_default().push(MaskRectangle {
                        corners: points,
                        color_style: preview_color,
                    });
                }
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
        drop(core);
        let mut chunks = Array::<VarDictionary>::new();
        for (chunk, rectangles) in masks {
            let mut payload = VarDictionary::new();
            payload.set("chunk", Vector2i::new(chunk.0, chunk.1));
            export_mask(&mut payload, chunk, &rectangles);
            chunks.push(&payload);
        }
        result.set("masks", chunks);
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
        let (applied, publication) = {
            let mut core = self.lock_core();
            let applied = core.apply_cell_selection_internal(&preview, profile);
            // The road tool's preview tests its pieces against the painted cells it publishes
            // (`ROAD-68` open question 25); unchanged roads publish in `O(1)`.
            (
                applied,
                applied.then(|| core.publish_road_tool_view()).flatten(),
            )
        };
        if let Some(publication) = publication {
            self.road_tool.publish(publication);
        }
        applied
    }
}

// Cell and parcel revisions already stamp every intersected mask chunk. Terrain geometry
// is the receiving surface, so only a whole-world reset contributes its epoch here.
fn mask_version(core: &crate::nodes::sim::core::SimCore, chunk: (i32, i32)) -> [i64; 3] {
    [
        core.zoning.cells.chunk_state(chunk).0 as i64,
        core.zoning.parcels.chunk_revision(chunk) as i64,
        core.terrain_payload_global_generation as i64,
    ]
}

fn mask_color(color: Color, style: f32) -> [f32; 4] {
    let linear = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    [linear(color.r), linear(color.g), linear(color.b), style]
}

fn export_mask(result: &mut VarDictionary, chunk: (i32, i32), rectangles: &[MaskRectangle]) {
    result.set("rectangle_count", rectangles.len() as i64);
    if rectangles.is_empty() {
        return;
    }
    let mask = build_mask(
        DVec2::new(f64::from(chunk.0), f64::from(chunk.1)) * 512.0,
        rectangles,
    );
    result.set("mask", PackedByteArray::from(mask.indices.as_slice()));
    let bytes: Vec<u8> = mask
        .rows
        .iter()
        .flatten()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    result.set("rows", PackedByteArray::from(bytes.as_slice()));
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
    for address in addresses.as_chunks::<3>().0 {
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
