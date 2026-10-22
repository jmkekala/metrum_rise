// SPDX-License-Identifier: GPL-2.0-only

//! Yard events: the hedge rows and planting areas a placed building's asset asks for, and the
//! buildings removed.
//!
//! The allocator only plans and queues; SimCore lays and removes the plants, because they are
//! vegetation edits and the allocator does not own vegetation.

use super::BuildingAllocator;
use super::entrance::{building_local_xz_basis, building_local_xz_pos};
use crate::assets::asset::{
    AnchorType, YardHedgeKind, YardLot, YardPlantKind, plan_yard_hedge, structure_footprint,
};
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::Vector2;

/// Stable identity of a zoned building's yard: its parcel and that parcel's build generation.
/// Building indices are swap-removed, so they cannot name what a yard hedge belongs to.
pub(crate) type YardKey = (u64, u32);

/// One planned hedge row in world metres, with whether each end may join a hedge it meets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct YardHedgeRowWorld {
    pub(crate) from: Vector2,
    pub(crate) to: Vector2,
    pub(crate) join_from: bool,
    pub(crate) join_to: bool,
}

/// One planting area in its lot's frame: `polygon` is asset-local `[x, z]`, and a local point
/// lands in the world at `origin + basis_x * x + basis_z * z`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct YardPlantingArea {
    pub(crate) plants: YardPlantKind,
    pub(crate) polygon: Vec<[f32; 2]>,
    pub(crate) origin: Vector2,
    pub(crate) basis_x: Vector2,
    pub(crate) basis_z: Vector2,
}

/// A yard change SimCore applies after the allocator step that made it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum YardEvent {
    /// A building was placed whose asset lines its yard with `hedge` along `rows` and plants
    /// `planting`; either may be absent.
    Placed {
        key: YardKey,
        hedge: Option<YardHedgeKind>,
        rows: Vec<YardHedgeRowWorld>,
        planting: Vec<YardPlantingArea>,
    },
    /// A zoned building was removed.
    Removed(YardKey),
}

// How far apart the front corners of two lots may stand and still share a side line: the wedge
// a curve leaves between rigid lots opens at most this wide at the street.
const SHARED_SIDE_FRONT_GAP_M: f32 = 4.0;
// Side lines turned further apart than this belong to lots on different streets, not a curve.
const SHARED_SIDE_MAX_TURN_COS: f32 = 0.866;
// How close a row end must lie to a lot line to count as standing on it; rows are planned in
// the asset's frame and moved into the world, so this only absorbs f32 rounding.
const ON_LOT_LINE_M: f32 = 0.05;

/// A lot side line and the line it shares with the neighbouring lot across a curve's wedge.
/// Lots are rigid rectangles, so on a curve two neighbours' side lines splay apart; each yard
/// moves its hedge onto the wedge's centre line instead, so both lay one hedge there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SharedLotSide {
    /// The lot's own side line, from its front corner to its rear corner.
    front: Vector2,
    rear: Vector2,
    /// The shared line: it starts midway between the two front corners and runs midway between
    /// the two side directions.
    shared_front: Vector2,
    shared_dir: Vector2,
}

/// The side lines of parcel `parcel_id` that a neighbouring lot across a curve shares, from the
/// parcel index. Lots on a straight street share their side lines already and come back
/// unchanged. Deterministic: both lots of a pair compute the same shared line, as the midpoint
/// and the direction sum are symmetric. O(parcels in the lot's chunks).
pub(crate) fn shared_lot_sides(zoning: &ZoningSystem, parcel_id: u64) -> Vec<SharedLotSide> {
    let Some(lot) = zoning.parcel_by_raw_id(parcel_id) else {
        return Vec::new();
    };
    // Corners run front-left, front-right, rear-right, rear-left.
    let sides = |c: [Vector2; 4]| [(c[0], c[3]), (c[1], c[2])];
    let pad = Vector2::splat(SHARED_SIDE_FRONT_GAP_M);
    let mut shared = Vec::new();
    for (front, rear) in sides(lot.corners()) {
        let Some(dir) = (rear - front).try_normalized() else {
            continue;
        };
        let outward = (front - lot.center()).dot(dir.orthogonal()).signum() * dir.orthogonal();
        let mut best: Option<(f32, u64, Vector2, Vector2)> = None;
        zoning
            .parcels
            .visit_in_bounds(front - pad, front + pad, |other| {
                if other.id().raw() == parcel_id || (other.center() - front).dot(outward) <= 0.0 {
                    return;
                }
                for (other_front, other_rear) in sides(other.corners()) {
                    let gap = other_front.distance_to(front);
                    let Some(other_dir) = (other_rear - other_front).try_normalized() else {
                        continue;
                    };
                    let key = (gap, other.id().raw());
                    if gap <= SHARED_SIDE_FRONT_GAP_M
                        && dir.dot(other_dir) >= SHARED_SIDE_MAX_TURN_COS
                        && best.is_none_or(|(g, id, _, _)| key < (g, id))
                    {
                        best = Some((gap, other.id().raw(), other_front, other_dir));
                    }
                }
            });
        let Some((_, _, other_front, other_dir)) = best else {
            continue;
        };
        let Some(shared_dir) = (dir + other_dir).try_normalized() else {
            continue;
        };
        shared.push(SharedLotSide {
            front,
            rear,
            shared_front: (front + other_front) * 0.5,
            shared_dir,
        });
    }
    shared
}

/// Moves a yard's hedge rows onto the side lines it shares (see [`SharedLotSide`]). A row along
/// a side moves onto the shared line at the same depth from the street; a row ending on a side
/// runs on, or stops short, to meet the shared line. Rows off the lot lines stay as planned.
/// O(rows * sides).
pub(crate) fn fit_rows_to_shared_sides(rows: &mut [YardHedgeRowWorld], sides: &[SharedLotSide]) {
    for side in sides {
        let Some(dir) = (side.rear - side.front).try_normalized() else {
            continue;
        };
        let on_side = |p: Vector2| (p - side.front).dot(dir.orthogonal()).abs() <= ON_LOT_LINE_M;
        for row in rows.iter_mut() {
            let (from_on, to_on) = (on_side(row.from), on_side(row.to));
            if from_on && to_on {
                let shared =
                    |p: Vector2| side.shared_front + side.shared_dir * (p - side.front).dot(dir);
                row.from = shared(row.from);
                row.to = shared(row.to);
                continue;
            }
            let meet = |end: Vector2, other: Vector2| {
                let along = end - other;
                let denom = along.cross(side.shared_dir);
                if denom.abs() <= f32::EPSILON {
                    return end;
                }
                other + along * ((side.shared_front - other).cross(side.shared_dir) / denom)
            };
            if from_on && row.join_from {
                row.from = meet(row.from, row.to);
            } else if to_on && row.join_to {
                row.to = meet(row.to, row.from);
            }
        }
    }
}

impl BuildingAllocator {
    /// Queues the yard of the building at `building_idx`, if its asset authors a hedge or
    /// planting areas and it stands on a parcel. O(rows * samples * surface and wall vertices +
    /// area vertices) from the asset's own geometry.
    pub(crate) fn queue_yard(&mut self, building_idx: usize, zone_cell_m: f32) {
        let Some(building) = self.buildings.get(building_idx) else {
            return;
        };
        let Some(manifest) = self
            .registry
            .get(&building.asset_id)
            .map(|entry| &entry.manifest)
        else {
            return;
        };
        let Some(data) = manifest.building.as_ref() else {
            return;
        };
        if building.parcel_id == 0 || (data.yard_hedge.is_none() && data.yard_planting.is_empty()) {
            return;
        }
        let frontage = manifest.building_frontage_forward();
        let rows = match &data.yard_hedge {
            Some(yard) => {
                let surfaces: Vec<Vec<[f32; 2]>> = manifest
                    .site_surfaces
                    .iter()
                    .map(|surface| surface.vertices.clone())
                    .collect();
                let structures: Vec<_> = manifest
                    .mesh_parts
                    .iter()
                    .filter_map(structure_footprint)
                    .collect();
                let entrance = manifest
                    .anchors
                    .iter()
                    .find(|anchor| {
                        anchor.anchor_type == AnchorType::Entrance && anchor.name == "main"
                    })
                    .map(|anchor| [anchor.position[0], anchor.position[2]]);
                let lot = YardLot {
                    half_width_m: f32::from(building.width_cells) * zone_cell_m * 0.5,
                    half_depth_m: f32::from(building.depth_cells) * zone_cell_m * 0.5,
                    frontage: [frontage[0], frontage[2]],
                    surfaces: &surfaces,
                    entrance,
                    structures: &structures,
                };
                let world =
                    |p: [f32; 2]| building_local_xz_pos(building, [p[0], 0.0, p[1]], frontage);
                plan_yard_hedge(&lot, &yard.edges)
                    .into_iter()
                    .map(|row| YardHedgeRowWorld {
                        from: world(row.from),
                        to: world(row.to),
                        join_from: row.join_from,
                        join_to: row.join_to,
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        let (basis_x, basis_z) = building_local_xz_basis(building.facing_dir, frontage);
        let origin = Vector2::new(building.center_x, building.center_y);
        let planting = data
            .yard_planting
            .iter()
            .map(|area| YardPlantingArea {
                plants: area.plants,
                polygon: area.vertices.clone(),
                origin,
                basis_x,
                basis_z,
            })
            .collect();
        let event = YardEvent::Placed {
            key: (building.parcel_id, building.build_generation),
            hedge: data.yard_hedge.as_ref().map(|yard| yard.hedge),
            rows,
            planting,
        };
        self.pending_yards.push(event);
    }

    // Queues the removal of a zoned building's yard. SimCore ignores a yard it laid no hedge for.
    pub(super) fn queue_yard_removal(&mut self, building_idx: usize) {
        if let Some(building) = self.buildings.get(building_idx)
            && building.parcel_id != 0
        {
            let key = (building.parcel_id, building.build_generation);
            self.pending_yards.push(YardEvent::Removed(key));
        }
    }
}
