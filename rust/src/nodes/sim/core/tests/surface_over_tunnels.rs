// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-61` round R8h (the user's decision, 2026-11-01): the land over a tunnel's bore is used as
//! if no tunnel ran under it. Explicit buildings, their pads and fields stand over a bore, and a
//! tunnel is drawn under a field.

use super::fields::{farm_fixture, farm_fixture_with};
use super::*;
use crate::nodes::sim::core::road_commit::{RoadCommitOutcome, RoadCommitRequest};

// A tunnel 6 m under the flat farm fixture, north from under its road, under the farm's lot and
// its field.
fn commit_tunnel(core: &mut SimCore) -> RoadCommitOutcome {
    core.precompute_road_mesh_data();
    core.refresh_road_terrain_inputs();
    core.commit_road(
        RoadCommitRequest {
            points: vec![
                Vector3::new(4.0, -6.0, -40.0),
                Vector3::new(4.0, -6.0, 300.0),
            ],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            tunnel: true,
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    )
}

// A farm placed over the bore stands as it does with no tunnel: placed (before, the bore's edge
// corridor refused the site as a road overlap), on the same pad bitwise (the site grading's road
// probes read the tunnel's buried deck as the visible road), and its field over the bore is
// valid and commits (before, the bore's buried span was a road under the field).
#[test]
fn a_farm_and_its_field_stand_over_a_tunnel() {
    let (mut core, farm, polygon) = farm_fixture_with(|core| {
        let outcome = commit_tunnel(core);
        assert!(outcome.committed, "{}", outcome.rejection);
    });
    let (bare, bare_farm, _) = farm_fixture();
    let (site, bare_site) = (
        &core.allocator.building_sites[farm],
        &bare.allocator.building_sites[bare_farm],
    );
    assert_eq!(
        site.support_height_m.to_bits(),
        bare_site.support_height_m.to_bits()
    );
    assert_eq!(site.footprint_world, bare_site.footprint_world);
    core.validate_field_polygon_internal(farm, &polygon)
        .unwrap();
    core.commit_field_polygon_internal(farm, polygon).unwrap();
}

// A tunnel is drawn under a committed field: its buried span covers no field (before, it was
// refused `field_overlap`).
#[test]
fn a_tunnel_is_drawn_under_a_field() {
    let (mut core, farm, polygon) = farm_fixture();
    core.commit_field_polygon_internal(farm, polygon).unwrap();
    let outcome = commit_tunnel(&mut core);
    assert!(outcome.committed, "{}", outcome.rejection);
}
