//! Spatial lookup and ray-query operations for derived building sites.

use super::geometry::{point_in_polygon_slice, update_site_plane_ray_hit};
use super::model::BuildingSiteClient;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use godot::prelude::{Vector2, Vector3};

impl BuildingSiteClient {
    pub(crate) fn contains_point(&self, pos: Vector2) -> bool {
        point_in_polygon_slice(pos, &self.footprint_world)
    }

    pub(super) fn height_at(&self, pos: Vector2) -> Option<f32> {
        for surface in &self.surfaces {
            if point_in_polygon_slice(pos, &surface.vertices_world) {
                return Some(surface.height_m);
            }
        }
        self.contains_point(pos).then_some(self.support_height_m)
    }

    pub(crate) fn raycast(&self, ray_origin: Vector3, ray_dir: Vector3) -> Option<Vector3> {
        if ray_dir.length_squared() <= f32::EPSILON {
            return None;
        }

        let mut best: Option<(f32, Vector3)> = None;
        for surface in &self.surfaces {
            update_site_plane_ray_hit(
                &mut best,
                ray_origin,
                ray_dir,
                surface.height_m,
                &surface.vertices_world,
            );
        }
        update_site_plane_ray_hit(
            &mut best,
            ray_origin,
            ray_dir,
            self.support_height_m,
            &self.footprint_world,
        );
        best.map(|(_, hit)| hit)
    }
}

impl BuildingAllocator {
    /// Rebuilds the derived site clients and shared building index when a site query needs them.
    pub(crate) fn prepare_building_site_query_index(&mut self, zone_cell_m: f32) {
        if self.building_sites.len() != self.buildings.len() {
            self.rebuild_building_site_clients(zone_cell_m);
        }
        if self.dirty_index || (self.building_chunks.is_empty() && !self.buildings.is_empty()) {
            self.rebuild_zone_index();
        }
    }

    pub(crate) fn site_world_bounds(&self, building_idx: usize) -> Option<(f32, f32, f32, f32)> {
        self.building_sites
            .get(building_idx)
            .map(|site| site.lot_bounds())
    }

    pub(crate) fn accumulate_pending_site_dirty_bounds(
        &mut self,
        bounds: Option<(f32, f32, f32, f32)>,
    ) {
        let Some(bounds) = bounds else {
            return;
        };
        if let Some(existing) = &mut self.building_site_dirty_bounds {
            existing.0 = existing.0.min(bounds.0);
            existing.1 = existing.1.min(bounds.1);
            existing.2 = existing.2.max(bounds.2);
            existing.3 = existing.3.max(bounds.3);
        } else {
            self.building_site_dirty_bounds = Some(bounds);
        }
    }

    pub(crate) fn take_pending_site_dirty_bounds(&mut self) -> Option<(f32, f32, f32, f32)> {
        self.building_site_dirty_bounds.take()
    }

    pub(crate) fn sample_building_site_height(&self, pos: Vector2) -> Option<f32> {
        if self.dirty_index
            || self.building_sites.len() != self.buildings.len()
            || self.building_chunks.is_empty()
        {
            return self
                .building_sites
                .iter()
                .find_map(|site| site.height_at(pos));
        }

        let margin_m = self.max_site_radius_m.max(0.0);
        let chunk_size = RegionGraph::CHUNK_SIZE;
        let min_chunk_x = ((pos.x - margin_m) / chunk_size).floor() as i32;
        let max_chunk_x = ((pos.x + margin_m) / chunk_size).floor() as i32;
        let min_chunk_z = ((pos.y - margin_m) / chunk_size).floor() as i32;
        let max_chunk_z = ((pos.y + margin_m) / chunk_size).floor() as i32;

        let mut best_idx = usize::MAX;
        let mut best_height = None;
        for chunk_x in min_chunk_x..=max_chunk_x {
            for chunk_z in min_chunk_z..=max_chunk_z {
                let Some(indices) = self.building_chunks.get(&(chunk_x, chunk_z)) else {
                    continue;
                };
                for &idx in indices {
                    if idx >= best_idx || idx >= self.building_sites.len() {
                        continue;
                    }
                    let Some(height) = self.building_sites[idx].height_at(pos) else {
                        continue;
                    };
                    best_idx = idx;
                    best_height = Some(height);
                }
            }
        }

        best_height
    }

    pub(crate) fn raycast_building_site_surface(
        &self,
        ray_origin: Vector3,
        ray_dir: Vector3,
        world_bounds: (f32, f32, f32, f32),
    ) -> Option<Vector3> {
        let ray_denom = ray_dir.length_squared();
        if !ray_origin.y.is_finite()
            || !ray_dir.y.is_finite()
            || !ray_denom.is_finite()
            || ray_denom <= f32::EPSILON
            || self.dirty_index
            || self.building_sites.len() != self.buildings.len()
            || self.building_chunks.is_empty()
        {
            return None;
        }

        let (min_x, min_z, max_x, max_z) = projected_ray_bounds_in_world(
            ray_origin,
            ray_dir,
            world_bounds.0,
            world_bounds.1,
            world_bounds.2,
            world_bounds.3,
        )?;
        let mut best: Option<(f32, usize, Vector3)> = None;
        self.visit_indexed_site_candidates_for_bounds(min_x, min_z, max_x, max_z, |building_idx| {
            let Some(site) = self.building_sites.get(building_idx) else {
                return;
            };
            let Some(hit) = site.raycast(ray_origin, ray_dir) else {
                return;
            };
            let t = (hit - ray_origin).dot(ray_dir) / ray_denom;
            if t < 0.0 {
                return;
            }
            if best.as_ref().is_none_or(|(best_t, best_idx, _)| {
                t < *best_t || (t == *best_t && building_idx < *best_idx)
            }) {
                best = Some((t, building_idx, hit));
            }
        });
        best.map(|(_, _, hit)| hit)
    }

    fn visit_indexed_site_candidates_for_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        mut visit: impl FnMut(usize),
    ) {
        let margin_m = self.max_site_radius_m.max(0.0);
        let chunk_size = RegionGraph::CHUNK_SIZE;
        let min_chunk_x = ((min_x.min(max_x) - margin_m) / chunk_size).floor() as i32;
        let max_chunk_x = ((min_x.max(max_x) + margin_m) / chunk_size).floor() as i32;
        let min_chunk_z = ((min_z.min(max_z) - margin_m) / chunk_size).floor() as i32;
        let max_chunk_z = ((min_z.max(max_z) + margin_m) / chunk_size).floor() as i32;

        for chunk_x in min_chunk_x..=max_chunk_x {
            for chunk_z in min_chunk_z..=max_chunk_z {
                let Some(indices) = self.building_chunks.get(&(chunk_x, chunk_z)) else {
                    continue;
                };
                for &building_idx in indices {
                    if building_idx < self.building_sites.len() {
                        visit(building_idx);
                    }
                }
            }
        }
    }

    /// Returns site indices whose chunk coverage may overlap the world bounds.
    pub(crate) fn site_candidate_indices_for_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Vec<usize> {
        if self.dirty_index
            || self.building_sites.len() != self.buildings.len()
            || self.building_chunks.is_empty()
        {
            return (0..self.building_sites.len()).collect();
        }

        let mut candidates = Vec::new();
        self.visit_indexed_site_candidates_for_bounds(min_x, min_z, max_x, max_z, |building_idx| {
            candidates.push(building_idx)
        });
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}

fn projected_ray_bounds_in_world(
    ray_origin: Vector3,
    ray_dir: Vector3,
    min_x: f32,
    min_z: f32,
    max_x: f32,
    max_z: f32,
) -> Option<(f32, f32, f32, f32)> {
    if !ray_origin.x.is_finite()
        || !ray_origin.z.is_finite()
        || !ray_dir.x.is_finite()
        || !ray_dir.z.is_finite()
    {
        return None;
    }

    let mut entry_t = f32::NEG_INFINITY;
    let mut exit_t = f32::INFINITY;
    clip_projected_ray_axis(
        ray_origin.x,
        ray_dir.x,
        min_x.min(max_x),
        min_x.max(max_x),
        &mut entry_t,
        &mut exit_t,
    )?;
    clip_projected_ray_axis(
        ray_origin.z,
        ray_dir.z,
        min_z.min(max_z),
        min_z.max(max_z),
        &mut entry_t,
        &mut exit_t,
    )?;
    if exit_t < entry_t || exit_t < 0.0 {
        return None;
    }
    if !entry_t.is_finite() || !exit_t.is_finite() {
        return Some((ray_origin.x, ray_origin.z, ray_origin.x, ray_origin.z));
    }

    let start_t = entry_t.max(0.0);
    let end_t = exit_t.max(start_t);
    let start = ray_origin + ray_dir * start_t;
    let end = ray_origin + ray_dir * end_t;
    Some((
        start.x.min(end.x),
        start.z.min(end.z),
        start.x.max(end.x),
        start.z.max(end.z),
    ))
}

fn clip_projected_ray_axis(
    origin: f32,
    direction: f32,
    min: f32,
    max: f32,
    entry_t: &mut f32,
    exit_t: &mut f32,
) -> Option<()> {
    if direction.abs() <= f32::EPSILON {
        return (origin >= min && origin <= max).then_some(());
    }

    let t0 = (min - origin) / direction;
    let t1 = (max - origin) / direction;
    *entry_t = entry_t.max(t0.min(t1));
    *exit_t = exit_t.min(t0.max(t1));
    Some(())
}
