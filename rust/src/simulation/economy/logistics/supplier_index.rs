//! Resource-keyed supplier lookup over existing building chunks.

use std::collections::BTreeMap;

use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{ResourceRuntimeId, RuntimeEconomyCatalog};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::ZoneType;
use godot::prelude::Vector3;
use rayon::prelude::*;

/// Resource-compatible supplier candidates grouped by chunk for bounded nearby lookup.
pub(super) struct SupplierCandidateIndex {
    by_resource_chunk: BTreeMap<ResourceRuntimeId, BTreeMap<(i32, i32), Vec<usize>>>,
}

impl SupplierCandidateIndex {
    pub(super) fn build(allocator: &BuildingAllocator, catalog: &RuntimeEconomyCatalog) -> Self {
        let mut entries: Vec<(ResourceRuntimeId, (i32, i32), usize)> = allocator
            .buildings
            .par_iter()
            .enumerate()
            .filter_map(|(idx, building)| {
                if building.broken
                    || building.economy_broken
                    || building.is_deserted
                    || building.edge_idx == usize::MAX
                    || !matches!(
                        building.zone_type,
                        ZoneType::Industrial | ZoneType::Commercial
                    )
                {
                    return None;
                }
                let Some(profile) =
                    catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                else {
                    return None;
                };
                let chunk = RegionGraph::get_chunk_coords(Vector3::new(
                    building.center_x,
                    0.0,
                    building.center_y,
                ));
                Some((idx, chunk, profile.outputs.as_slice()))
            })
            .flat_map_iter(|(idx, chunk, outputs)| {
                outputs
                    .iter()
                    .map(move |output| (output.resource_runtime_id, chunk, idx))
            })
            .collect();
        entries.sort_unstable();

        let mut by_resource_chunk = BTreeMap::new();
        for (resource_runtime_id, chunk, idx) in entries {
            by_resource_chunk
                .entry(resource_runtime_id)
                .or_insert_with(BTreeMap::new)
                .entry(chunk)
                .or_insert_with(Vec::new)
                .push(idx);
        }

        Self { by_resource_chunk }
    }

    pub(super) fn fill_nearby_candidates(
        &self,
        resource_runtime_id: ResourceRuntimeId,
        origin_x: f32,
        origin_y: f32,
        max_chunk_radius: i32,
        candidate_limit: usize,
        allocator: &BuildingAllocator,
        candidates: &mut Vec<usize>,
        mut eligible: impl FnMut(usize, &Building) -> bool,
    ) {
        candidates.clear();
        if candidate_limit == 0 {
            return;
        }
        let Some(by_chunk) = self.by_resource_chunk.get(&resource_runtime_id) else {
            return;
        };
        let origin_chunk = RegionGraph::get_chunk_coords(Vector3::new(origin_x, 0.0, origin_y));

        for ring in 0..=max_chunk_radius {
            for dx in -ring..=ring {
                for dz in -ring..=ring {
                    if ring > 0 && dx.abs() != ring && dz.abs() != ring {
                        continue;
                    }
                    let chunk_key = (origin_chunk.0 + dx, origin_chunk.1 + dz);
                    let Some(indices) = by_chunk.get(&chunk_key) else {
                        continue;
                    };
                    for &idx in indices {
                        if idx >= allocator.buildings.len() {
                            continue;
                        }
                        if eligible(idx, &allocator.buildings[idx]) {
                            candidates.push(idx);
                        }
                    }
                }
            }
        }

        candidates.sort_unstable_by(|&a, &b| {
            let da = squared_distance(origin_x, origin_y, &allocator.buildings[a]);
            let db = squared_distance(origin_x, origin_y, &allocator.buildings[b]);
            da.total_cmp(&db).then_with(|| a.cmp(&b))
        });
        candidates.truncate(candidate_limit);
    }
}

fn squared_distance(origin_x: f32, origin_y: f32, building: &Building) -> f32 {
    let dx = building.center_x - origin_x;
    let dy = building.center_y - origin_y;
    dx * dx + dy * dy
}
