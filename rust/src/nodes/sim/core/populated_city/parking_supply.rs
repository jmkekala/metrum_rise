// SPDX-License-Identifier: GPL-2.0-only

//! The populated fixture's parking supply (`PARK-02`, F8): kerbside lanes on the cross streets,
//! none on the rows or the trunk, small lots along the rows, and every door with bays to walk to.

use super::{CROSS_SPACING_M, ROW_BLOCKS_PER_LOT, zoned_city};
use crate::simulation::network::graph::{RoadParking, SidewalkSides};
use crate::simulation::parking::PlaceSource;

#[test]
fn the_fixture_parks_on_its_cross_streets_and_in_lots_along_its_rows() {
    let side_m = 2.0 * CROSS_SPACING_M;
    let core = zoned_city(side_m);
    let graph = &core.region_graph;
    let (mut cross, mut rows) = (0, 0);
    for id in (0..graph.edge_count()).filter(|&id| !graph.edge(id).deleted) {
        let edge = graph.edge(id);
        let (start, end) = (graph.node(edge.start_node).pos, graph.node(edge.end_node).pos);
        if (start.x - end.x).abs() <= 1.0 {
            cross += 1;
            assert!(edge.parking.left && edge.parking.right, "cross street {id}");
            assert_eq!(edge.sidewalk_sides(), SidewalkSides::BOTH);
        } else {
            rows += 1;
            assert_eq!(edge.parking, RoadParking::NONE, "row {id}");
        }
    }
    assert!(cross > 0 && rows > 0);
    let kerb: usize = (0..graph.edge_count() as u32)
        .flat_map(|edge| [(edge, false), (edge, true)])
        .map(|(edge, right)| core.parking.source_bays(PlaceSource::Kerb { edge, right }).len())
        .sum();
    assert!(kerb > 0, "no kerbside bay");
    let is_lot = |index: &usize| core.allocator.buildings[*index].asset_id.ends_with("small_lot");
    let lots: Vec<_> = (0..core.allocator.buildings.len()).filter(is_lot).collect();
    for &lot in &lots {
        let handle = core.allocator.building_handles.handle(lot);
        let issues = core.parking.diagnostics.get(&handle.to_bits());
        assert!(issues.is_none(), "{issues:?}");
    }
    let small_lots = lots.len();
    let row_blocks = 2 * (side_m / 50.0 + 1.0).floor() as usize;
    assert_eq!(small_lots, row_blocks.div_ceil(ROW_BLOCKS_PER_LOT));
    let bays = core.parking.counts().0;
    assert!(bays >= kerb + 12 * small_lots, "{bays} bays, {kerb} at the kerb, {small_lots} lots");
    for (index, building) in core.allocator.buildings.iter().enumerate() {
        let handle = core.allocator.building_handles.handle(index);
        assert!(
            !core.parking.candidate_products(handle.to_bits()).is_empty(),
            "{} at ({}, {}) has no bay within its walk",
            building.asset_id,
            building.center_x,
            building.center_y
        );
    }
}
