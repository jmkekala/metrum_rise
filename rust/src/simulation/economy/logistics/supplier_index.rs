//! Resource-keyed supplier lookup over freight-reachable components.

use std::collections::BTreeMap;

use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::accessibility::{
    BuildingModeComponents, ModeComponentIndex, NO_COMPONENT,
};
use crate::simulation::economy::definitions::{ResourceRuntimeId, RuntimeEconomyCatalog};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

/// Resource-compatible supplier candidates grouped by freight-reachable component.
pub(super) struct SupplierCandidateIndex {
    by_resource_component: BTreeMap<ResourceRuntimeId, BTreeMap<u32, Vec<usize>>>,
}

impl SupplierCandidateIndex {
    pub(super) fn build(
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        freight_components: &ModeComponentIndex,
    ) -> Self {
        let mut entries: Vec<(ResourceRuntimeId, u32, usize)> = allocator
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
                let components = freight_components.building_components(
                    allocator,
                    graph,
                    idx,
                    TransitFlags::CAR,
                );
                if components.as_slice().is_empty() {
                    return None;
                }
                Some((idx, components, profile.outputs.as_slice()))
            })
            .flat_map_iter(|(idx, components, outputs)| {
                let (component_values, component_count) = components.raw_parts();
                outputs.iter().flat_map(move |output| {
                    (0..component_count).filter_map(move |component_idx| {
                        let component = component_values[component_idx];
                        (component != NO_COMPONENT).then_some((
                            output.resource_runtime_id,
                            component,
                            idx,
                        ))
                    })
                })
            })
            .collect();
        entries.sort_unstable();

        let mut by_resource_component = BTreeMap::new();
        for (resource_runtime_id, component, idx) in entries {
            by_resource_component
                .entry(resource_runtime_id)
                .or_insert_with(BTreeMap::new)
                .entry(component)
                .or_insert_with(Vec::new)
                .push(idx);
        }

        Self {
            by_resource_component,
        }
    }

    pub(super) fn fill_reachable_candidates(
        &self,
        resource_runtime_id: ResourceRuntimeId,
        destination_idx: usize,
        origin_x: f32,
        origin_y: f32,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        freight_components: &ModeComponentIndex,
        candidates: &mut Vec<usize>,
        mut eligible: impl FnMut(usize, &Building) -> bool,
    ) {
        candidates.clear();
        let Some(by_component) = self.by_resource_component.get(&resource_runtime_id) else {
            return;
        };
        let destination_components = freight_components.building_components(
            allocator,
            graph,
            destination_idx,
            TransitFlags::CAR,
        );
        extend_component_candidates(by_component, destination_components, candidates);
        candidates.sort_unstable();
        candidates.dedup();
        candidates.retain(|&idx| {
            idx < allocator.buildings.len() && eligible(idx, &allocator.buildings[idx])
        });

        candidates.sort_unstable_by(|&a, &b| {
            let da = squared_distance(origin_x, origin_y, &allocator.buildings[a]);
            let db = squared_distance(origin_x, origin_y, &allocator.buildings[b]);
            da.total_cmp(&db).then_with(|| a.cmp(&b))
        });
    }
}

fn extend_component_candidates(
    source: &BTreeMap<u32, Vec<usize>>,
    components: BuildingModeComponents,
    candidates: &mut Vec<usize>,
) {
    for &component in components.as_slice() {
        if component == NO_COMPONENT {
            continue;
        }
        if let Some(indices) = source.get(&component) {
            candidates.extend(indices.iter().copied());
        }
    }
}

fn squared_distance(origin_x: f32, origin_y: f32, building: &Building) -> f32 {
    let dx = building.center_x - origin_x;
    let dy = building.center_y - origin_y;
    dx * dx + dy * dy
}
