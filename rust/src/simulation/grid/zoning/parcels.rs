//! Road-aligned zoning parcel storage and placement geometry.

use crate::simulation::network::graph::RegionGraph;
use godot::prelude::{Vector2, Vector3};
use std::collections::{HashMap, HashSet};

const OVERLAP_EPSILON_M: f32 = 0.001;

/// Stable parcel identifier persisted in saves and referenced by buildings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParcelId(u64);

impl ParcelId {
    /// Reserved non-parcel value used by legacy or explicit non-zoned buildings.
    pub const NONE: Self = Self(0);

    /// Creates a parcel id from its persisted integer representation.
    pub fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// Returns the persisted integer representation of this parcel id.
    pub fn raw(self) -> u64 {
        self.0
    }

    /// Returns true when this is the reserved non-parcel value.
    pub fn is_none(self) -> bool {
        self.0 == 0
    }
}

/// User-authored road-aligned lot used as zoning and building-spawn authority.
#[derive(Clone, Debug)]
pub struct ZoningParcel {
    id: ParcelId,
    edge_idx: usize,
    side: i8,
    frontage_center_t: f32,
    frontage_m: f32,
    depth_m: f32,
    zone_profile_runtime_id: u16,
    occupied_building: Option<usize>,
    front_center: Vector2,
    center: Vector2,
    tangent: Vector2,
    normal: Vector2,
    corners: [Vector2; 4],
    aabb_min: Vector2,
    aabb_max: Vector2,
}

impl ZoningParcel {
    /// Returns the stable parcel id.
    pub fn id(&self) -> ParcelId {
        self.id
    }

    /// Returns the road edge this parcel fronts.
    pub fn edge_idx(&self) -> usize {
        self.edge_idx
    }

    /// Returns the side of the road this parcel occupies: `1` or `-1`.
    pub fn side(&self) -> i8 {
        self.side
    }

    /// Returns the center `t` coordinate of the parcel frontage on its road edge.
    pub fn frontage_center_t(&self) -> f32 {
        self.frontage_center_t
    }

    /// Returns the frontage width in metres.
    pub fn frontage_m(&self) -> f32 {
        self.frontage_m
    }

    /// Returns the parcel depth in metres.
    pub fn depth_m(&self) -> f32 {
        self.depth_m
    }

    /// Returns the assigned zoning-profile runtime id, or `0` for free/unzoned parcels.
    pub fn zone_profile_runtime_id(&self) -> u16 {
        self.zone_profile_runtime_id
    }

    /// Returns the occupied building index when a private zoned building claims this parcel.
    pub fn occupied_building(&self) -> Option<usize> {
        self.occupied_building
    }

    /// Returns true when no building currently claims this parcel.
    pub fn is_available(&self) -> bool {
        self.occupied_building.is_none()
    }

    /// Returns the world-space center of the parcel frontage line.
    pub fn front_center(&self) -> Vector2 {
        self.front_center
    }

    /// Returns the world-space center of the parcel rectangle.
    pub fn center(&self) -> Vector2 {
        self.center
    }

    /// Returns the unit vector along the road frontage.
    pub fn tangent(&self) -> Vector2 {
        self.tangent
    }

    /// Returns the unit vector pointing from the road into the parcel.
    pub fn normal(&self) -> Vector2 {
        self.normal
    }

    /// Returns parcel corners in front-left, front-right, rear-right, rear-left order.
    pub fn corners(&self) -> [Vector2; 4] {
        self.corners
    }

    /// Returns the minimum XZ corner of the parcel AABB.
    pub fn aabb_min(&self) -> Vector2 {
        self.aabb_min
    }

    /// Returns the maximum XZ corner of the parcel AABB.
    pub fn aabb_max(&self) -> Vector2 {
        self.aabb_max
    }

    pub(crate) fn set_zone_profile_runtime_id(&mut self, runtime_id: u16) {
        self.zone_profile_runtime_id = runtime_id;
    }

    pub(crate) fn set_occupied_building(&mut self, building_idx: Option<usize>) {
        self.occupied_building = building_idx;
    }

    pub(crate) fn set_edge_idx(&mut self, edge_idx: usize) {
        self.edge_idx = edge_idx;
    }
}

/// Geometry for a parcel that has been projected onto a road edge.
#[derive(Clone, Copy, Debug)]
pub struct ParcelGeometry {
    /// Road edge index that owns the parcel frontage.
    pub edge_idx: usize,
    /// Road side: `1` or `-1`.
    pub side: i8,
    /// Center `t` coordinate along the road edge.
    pub frontage_center_t: f32,
    /// Parcel frontage width in metres.
    pub frontage_m: f32,
    /// Parcel depth in metres.
    pub depth_m: f32,
    /// World-space center of the parcel frontage line.
    pub front_center: Vector2,
    /// World-space center of the parcel rectangle.
    pub center: Vector2,
    /// Unit vector along the road frontage.
    pub tangent: Vector2,
    /// Unit vector pointing from the road into the parcel.
    pub normal: Vector2,
    /// Corners in front-left, front-right, rear-right, rear-left order.
    pub corners: [Vector2; 4],
    /// Minimum XZ corner of the parcel AABB.
    pub aabb_min: Vector2,
    /// Maximum XZ corner of the parcel AABB.
    pub aabb_max: Vector2,
}

/// Reason a parcel placement request was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParcelPlacementError {
    /// The selected profile id was not present in the zoning-profile registry.
    UnknownProfile,
    /// No buildable road edge was close enough to attach the requested parcel.
    NoRoadAttachment,
    /// The projected frontage would extend beyond the road edge ends.
    FrontageOutOfBounds,
    /// The requested parcel dimensions are outside the supported first-slice edit range.
    InvalidDimensions,
    /// The requested rectangle overlaps an existing parcel.
    OverlapsExistingParcel,
    /// One or more parcel corners would sit outside the authored world extent.
    OutsideWorld,
}

/// Stable parcel collection plus its coarse chunk index.
#[derive(Clone, Debug)]
pub struct ParcelStore {
    parcels: Vec<ZoningParcel>,
    id_to_index: HashMap<ParcelId, usize>,
    chunk_index: HashMap<(i32, i32), Vec<ParcelId>>,
    next_id: u64,
}

impl Default for ParcelStore {
    fn default() -> Self {
        Self {
            parcels: Vec::new(),
            id_to_index: HashMap::new(),
            chunk_index: HashMap::new(),
            next_id: 1,
        }
    }
}

impl ParcelStore {
    /// Returns every parcel in stable storage order.
    pub fn parcels(&self) -> &[ZoningParcel] {
        &self.parcels
    }

    /// Returns the parcel for one stable id.
    pub fn get(&self, id: ParcelId) -> Option<&ZoningParcel> {
        let index = *self.id_to_index.get(&id)?;
        self.parcels.get(index)
    }

    /// Returns a mutable parcel reference for one stable id.
    pub fn get_mut(&mut self, id: ParcelId) -> Option<&mut ZoningParcel> {
        let index = *self.id_to_index.get(&id)?;
        self.parcels.get_mut(index)
    }

    /// Removes all parcels and resets id allocation.
    pub fn clear(&mut self) {
        self.parcels.clear();
        self.id_to_index.clear();
        self.chunk_index.clear();
        self.next_id = 1;
    }

    pub(crate) fn insert_new(
        &mut self,
        geometry: ParcelGeometry,
        zone_profile_runtime_id: u16,
    ) -> ParcelId {
        let id = ParcelId(self.next_id);
        self.next_id = self.next_id.saturating_add(1).max(1);
        self.insert_with_id(id, geometry, zone_profile_runtime_id);
        id
    }

    pub(crate) fn insert_loaded(
        &mut self,
        id: ParcelId,
        geometry: ParcelGeometry,
        zone_profile_runtime_id: u16,
    ) {
        self.next_id = self.next_id.max(id.raw().saturating_add(1)).max(1);
        self.insert_with_id(id, geometry, zone_profile_runtime_id);
    }

    fn insert_with_id(
        &mut self,
        id: ParcelId,
        geometry: ParcelGeometry,
        zone_profile_runtime_id: u16,
    ) {
        let parcel = ZoningParcel {
            id,
            edge_idx: geometry.edge_idx,
            side: geometry.side,
            frontage_center_t: geometry.frontage_center_t,
            frontage_m: geometry.frontage_m,
            depth_m: geometry.depth_m,
            zone_profile_runtime_id,
            occupied_building: None,
            front_center: geometry.front_center,
            center: geometry.center,
            tangent: geometry.tangent,
            normal: geometry.normal,
            corners: geometry.corners,
            aabb_min: geometry.aabb_min,
            aabb_max: geometry.aabb_max,
        };
        let index = self.parcels.len();
        self.parcels.push(parcel);
        self.id_to_index.insert(id, index);
        self.index_parcel(index);
    }

    pub(crate) fn remove_edges_not_in_mapping(&mut self, mapping: &HashMap<usize, usize>) {
        let mut changed = false;
        for parcel in &mut self.parcels {
            if let Some(&new_idx) = mapping.get(&parcel.edge_idx) {
                parcel.set_edge_idx(new_idx);
            } else {
                parcel.set_edge_idx(usize::MAX);
                changed = true;
            }
        }
        let before = self.parcels.len();
        self.parcels.retain(|parcel| parcel.edge_idx != usize::MAX);
        changed |= self.parcels.len() != before;
        if changed {
            self.id_to_index.clear();
            for (idx, parcel) in self.parcels.iter().enumerate() {
                self.id_to_index.insert(parcel.id, idx);
            }
            self.rebuild_chunk_index();
        }
    }

    pub(crate) fn find_at_point(&self, point: Vector2) -> Option<ParcelId> {
        let chunk = chunk_key(point);
        let ids = self.chunk_index.get(&chunk)?;
        ids.iter().copied().find(|&id| {
            self.get(id)
                .map(|parcel| point_inside_parcel(point, parcel))
                .unwrap_or(false)
        })
    }

    pub(crate) fn overlaps_existing(&self, geometry: &ParcelGeometry) -> bool {
        let mut visited = HashSet::new();
        for chunk in chunks_for_aabb(geometry.aabb_min, geometry.aabb_max) {
            let Some(ids) = self.chunk_index.get(&chunk) else {
                continue;
            };
            for &id in ids {
                if !visited.insert(id) {
                    continue;
                }
                let Some(parcel) = self.get(id) else {
                    continue;
                };
                if rectangles_overlap_geometry(geometry, parcel) {
                    return true;
                }
            }
        }
        false
    }

    pub(crate) fn set_zone_profile_runtime_id(&mut self, id: ParcelId, runtime_id: u16) -> bool {
        let Some(parcel) = self.get_mut(id) else {
            return false;
        };
        parcel.set_zone_profile_runtime_id(runtime_id);
        true
    }

    pub(crate) fn set_occupied_building(&mut self, id: ParcelId, building_idx: usize) -> bool {
        let Some(parcel) = self.get_mut(id) else {
            return false;
        };
        if parcel.occupied_building.is_some() {
            return false;
        }
        parcel.set_occupied_building(Some(building_idx));
        true
    }

    pub(crate) fn clear_occupied_building(&mut self, id: ParcelId) -> bool {
        let Some(parcel) = self.get_mut(id) else {
            return false;
        };
        parcel.set_occupied_building(None);
        true
    }

    pub(crate) fn remap_occupied_building(&mut self, old_idx: usize, new_idx: usize) {
        for parcel in &mut self.parcels {
            if parcel.occupied_building == Some(old_idx) {
                parcel.occupied_building = Some(new_idx);
            }
        }
    }

    pub(crate) fn clear_all_occupancy(&mut self) {
        for parcel in &mut self.parcels {
            parcel.occupied_building = None;
        }
    }

    fn index_parcel(&mut self, index: usize) {
        let parcel = &self.parcels[index];
        for chunk in chunks_for_aabb(parcel.aabb_min, parcel.aabb_max) {
            self.chunk_index.entry(chunk).or_default().push(parcel.id);
        }
    }

    fn rebuild_chunk_index(&mut self) {
        self.chunk_index.clear();
        for idx in 0..self.parcels.len() {
            self.index_parcel(idx);
        }
    }
}

pub(crate) fn project_default_parcel_at(
    graph: &RegionGraph,
    world_pos: Vector2,
    frontage_m: f32,
    depth_m: f32,
) -> Result<ParcelGeometry, ParcelPlacementError> {
    let search_radius = depth_m + frontage_m + 48.0;
    let nearby_edges =
        graph.get_edges_near_point(Vector3::new(world_pos.x, 0.0, world_pos.y), search_radius);
    let mut best: Option<ProjectedRoadPoint> = None;

    for edge_idx in nearby_edges {
        let edge = graph.edge(edge_idx);
        if edge.deleted
            || edge.no_building_spawn
            || edge.physical_length <= frontage_m
            || edge.physical_geometry.len() < 2
        {
            continue;
        }
        let Some(projected) = project_point_to_edge(graph, edge_idx, world_pos) else {
            continue;
        };
        let max_centerline_dist = edge.width * 0.5 + crate::config::SIDEWALK_WIDTH + depth_m + 8.0;
        if projected.dist_m > max_centerline_dist {
            continue;
        }
        let better = best
            .as_ref()
            .map(|current| projected.dist_m < current.dist_m)
            .unwrap_or(true);
        if better {
            best = Some(projected);
        }
    }

    let Some(projected) = best else {
        return Err(ParcelPlacementError::NoRoadAttachment);
    };
    if projected.s_m < frontage_m * 0.5 || projected.s_m > projected.edge_len_m - frontage_m * 0.5 {
        return Err(ParcelPlacementError::FrontageOutOfBounds);
    }

    Ok(geometry_from_attachment(
        graph,
        projected.edge_idx,
        projected.side,
        projected.s_m / projected.edge_len_m,
        frontage_m,
        depth_m,
    ))
}

pub(crate) fn geometry_from_attachment(
    graph: &RegionGraph,
    edge_idx: usize,
    side: i8,
    frontage_center_t: f32,
    frontage_m: f32,
    depth_m: f32,
) -> ParcelGeometry {
    let edge = graph.edge(edge_idx);
    let s_m = frontage_center_t.clamp(0.0, 1.0) * edge.physical_length;
    let road_pos = sample_pos_on_polyline(&edge.physical_geometry, edge.physical_length, s_m);
    let tangent = sample_tangent_on_polyline(&edge.physical_geometry, edge.physical_length, s_m);
    let normal = Vector2::new(tangent.y, -tangent.x) * side as f32;
    let front_center = road_pos + normal * (edge.width * 0.5 + crate::config::SIDEWALK_WIDTH);
    let center = front_center + normal * (depth_m * 0.5);
    let half_frontage = frontage_m * 0.5;
    let front_left = front_center - tangent * half_frontage;
    let front_right = front_center + tangent * half_frontage;
    let rear_right = front_right + normal * depth_m;
    let rear_left = front_left + normal * depth_m;
    let corners = [front_left, front_right, rear_right, rear_left];
    let (aabb_min, aabb_max) = aabb_for_corners(&corners);
    ParcelGeometry {
        edge_idx,
        side,
        frontage_center_t,
        frontage_m,
        depth_m,
        front_center,
        center,
        tangent,
        normal,
        corners,
        aabb_min,
        aabb_max,
    }
}

pub(crate) fn geometry_inside_world(
    geometry: &ParcelGeometry,
    world_width_m: f32,
    world_height_m: f32,
) -> bool {
    let half_w = world_width_m * 0.5;
    let half_h = world_height_m * 0.5;
    geometry.corners.iter().all(|corner| {
        corner.x >= -half_w && corner.x <= half_w && corner.y >= -half_h && corner.y <= half_h
    })
}

fn point_inside_parcel(point: Vector2, parcel: &ZoningParcel) -> bool {
    let rel = point - parcel.center;
    let along = rel.dot(parcel.tangent);
    let depth = rel.dot(parcel.normal);
    along.abs() <= parcel.frontage_m * 0.5 + OVERLAP_EPSILON_M
        && depth.abs() <= parcel.depth_m * 0.5 + OVERLAP_EPSILON_M
}

fn rectangles_overlap_geometry(geometry: &ParcelGeometry, parcel: &ZoningParcel) -> bool {
    let axes = [
        geometry.tangent,
        geometry.normal,
        parcel.tangent,
        parcel.normal,
    ];
    axes.into_iter().all(|axis| {
        let (a_min, a_max) = project_corners(&geometry.corners, axis);
        let (b_min, b_max) = project_corners(&parcel.corners, axis);
        a_max > b_min + OVERLAP_EPSILON_M && b_max > a_min + OVERLAP_EPSILON_M
    })
}

fn project_corners(corners: &[Vector2; 4], axis: Vector2) -> (f32, f32) {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for corner in corners {
        let projected = corner.dot(axis);
        min = min.min(projected);
        max = max.max(projected);
    }
    (min, max)
}

fn aabb_for_corners(corners: &[Vector2; 4]) -> (Vector2, Vector2) {
    let mut min = Vector2::new(f32::INFINITY, f32::INFINITY);
    let mut max = Vector2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
    for corner in corners {
        min.x = min.x.min(corner.x);
        min.y = min.y.min(corner.y);
        max.x = max.x.max(corner.x);
        max.y = max.y.max(corner.y);
    }
    (min, max)
}

#[derive(Clone, Copy)]
struct ProjectedRoadPoint {
    edge_idx: usize,
    side: i8,
    s_m: f32,
    edge_len_m: f32,
    dist_m: f32,
}

fn project_point_to_edge(
    graph: &RegionGraph,
    edge_idx: usize,
    point: Vector2,
) -> Option<ProjectedRoadPoint> {
    let edge = graph.edge(edge_idx);
    if edge.physical_geometry.len() < 2 || edge.physical_length <= 1e-6 {
        return None;
    }

    let mut best_dist2 = f32::INFINITY;
    let mut best_s = 0.0;
    let mut best_tangent = Vector2::RIGHT;
    let mut accumulated = 0.0;

    for window in edge.physical_geometry.windows(2) {
        let p0 = Vector2::new(window[0].x, window[0].z);
        let p1 = Vector2::new(window[1].x, window[1].z);
        let segment = p1 - p0;
        let len2 = segment.length_squared();
        let seg_len = window[0].distance_to(window[1]);
        if len2 <= 1e-12 || seg_len <= 1e-6 {
            continue;
        }
        let local_t = ((point - p0).dot(segment) / len2).clamp(0.0, 1.0);
        let closest = p0 + segment * local_t;
        let dist2 = (point - closest).length_squared();
        if dist2 < best_dist2 {
            best_dist2 = dist2;
            best_s = accumulated + seg_len * local_t;
            best_tangent = segment.normalized();
        }
        accumulated += seg_len;
    }

    if !best_dist2.is_finite() {
        return None;
    }
    let side_one_normal = Vector2::new(best_tangent.y, -best_tangent.x);
    let side = if (point
        - sample_pos_on_polyline(&edge.physical_geometry, edge.physical_length, best_s))
    .dot(side_one_normal)
        >= 0.0
    {
        1
    } else {
        -1
    };
    Some(ProjectedRoadPoint {
        edge_idx,
        side,
        s_m: best_s,
        edge_len_m: edge.physical_length,
        dist_m: best_dist2.sqrt(),
    })
}

fn sample_pos_on_polyline(points: &[Vector3], total_len: f32, s_m: f32) -> Vector2 {
    if points.is_empty() {
        return Vector2::ZERO;
    }
    if points.len() == 1 || total_len <= 1e-6 {
        return Vector2::new(points[0].x, points[0].z);
    }

    let target_s = s_m.clamp(0.0, total_len);
    let mut acc_len = 0.0;
    for window in points.windows(2) {
        let seg_len = window[0].distance_to(window[1]);
        if seg_len <= 1e-6 {
            continue;
        }
        if acc_len + seg_len >= target_s {
            let local_t = ((target_s - acc_len) / seg_len).clamp(0.0, 1.0);
            let p0 = Vector2::new(window[0].x, window[0].z);
            let p1 = Vector2::new(window[1].x, window[1].z);
            return p0.lerp(p1, local_t);
        }
        acc_len += seg_len;
    }
    let last = points.last().unwrap();
    Vector2::new(last.x, last.z)
}

fn sample_tangent_on_polyline(points: &[Vector3], total_len: f32, s_m: f32) -> Vector2 {
    if points.len() <= 1 || total_len <= 1e-6 {
        return Vector2::RIGHT;
    }

    let target_s = s_m.clamp(0.0, total_len);
    let mut acc_len = 0.0;
    for window in points.windows(2) {
        let seg = Vector2::new(window[1].x - window[0].x, window[1].z - window[0].z);
        let seg_len = window[0].distance_to(window[1]);
        if seg_len <= 1e-6 || seg.length_squared() <= 1e-12 {
            continue;
        }
        if acc_len + seg_len >= target_s {
            return seg.normalized();
        }
        acc_len += seg_len;
    }

    for window in points.windows(2).rev() {
        let seg = Vector2::new(window[1].x - window[0].x, window[1].z - window[0].z);
        if seg.length_squared() > 1e-12 {
            return seg.normalized();
        }
    }
    Vector2::RIGHT
}

fn chunk_key(point: Vector2) -> (i32, i32) {
    (
        (point.x / RegionGraph::CHUNK_SIZE).floor() as i32,
        (point.y / RegionGraph::CHUNK_SIZE).floor() as i32,
    )
}

fn chunks_for_aabb(min: Vector2, max: Vector2) -> Vec<(i32, i32)> {
    let min_chunk = chunk_key(min);
    let max_chunk = chunk_key(max);
    let mut chunks = Vec::new();
    for cx in min_chunk.0..=max_chunk.0 {
        for cz in min_chunk.1..=max_chunk.1 {
            chunks.push((cx, cz));
        }
    }
    chunks
}
