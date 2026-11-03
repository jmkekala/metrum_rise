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
            median: None,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
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

// The 1:4 hill 25 m high with a tunnel through it (`road_tunnels::THROUGH_HILL`), its west
// approach in a walled trench from x = -100 to the portal at -76; with `west_road`, a grounded
// road from x = -125 on the flat ends where the tunnel stroke starts, so it continues the
// approach.
fn trench_core(west_road: bool) -> SimCore {
    use super::road_tunnels::{THROUGH_HILL, cone_hill_core};
    let mut core = cone_hill_core(100.0, 0.25);
    let commit = |core: &mut SimCore, points: Vec<Vector3>, tunnel: bool| {
        core.precompute_road_mesh_data();
        core.refresh_road_terrain_inputs();
        let outcome = core.commit_road(
            RoadCommitRequest {
                points,
                fwd_lanes: 1,
                bkw_lanes: 1,
                snap_to_existing_roads: true,
                tunnel,
                median: None,
                attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
                preview: None,
            },
            std::time::Instant::now(),
            Default::default(),
        );
        assert!(outcome.committed, "{}", outcome.rejection);
    };
    if west_road {
        commit(
            &mut core,
            vec![Vector3::new(-125.0, 0.0, 0.0), THROUGH_HILL[0]],
            false,
        );
    }
    commit(&mut core, THROUGH_HILL.to_vec(), true);
    core.precompute_road_mesh_data();
    core
}

// `ROAD-61` round R8h (the user's rule, 2026-11-01): a tunnel's entry is the exception to
// frontage. No zoning cell lines the approach's trench, and painting there finds no cell; the
// grounded road it continues keeps its cells. Before, the approach was an ordinary standard
// road and the generator lined its trench walls with cells on both sides.
#[test]
fn no_zoning_cells_line_a_tunnel_approach() {
    use crate::simulation::zoning::cells::CellSelectionShape;
    use glam::DVec2;
    let mut core = trench_core(true);
    core.preview_cell_selection_internal(
        CellSelectionShape::Brush { radius_m: 200.0 },
        &[DVec2::ZERO],
    );
    let cells = &core.zoning.cells;
    let along = |x0: f64, x1: f64| {
        let mut found = 0;
        let mut x = x0;
        while x <= x1 {
            for z in [-25.0, -15.0, -9.0, 9.0, 15.0, 25.0] {
                found += usize::from(cells.pick(DVec2::new(x, z)).is_some());
            }
            x += 4.0;
        }
        found
    };
    assert_eq!(along(-96.0, -80.0), 0, "cells along the approach's trench");
    assert!(
        along(-121.0, -106.0) > 10,
        "the continued road keeps its cells"
    );
    let selection = core.zoning.cells.select(
        CellSelectionShape::Brush { radius_m: 12.0 },
        &[DVec2::new(-88.0, 15.0)],
    );
    let painted =
        !selection.cells.is_empty() && core.zoning.paint_cells(&selection, 1, |_| false).is_some();
    assert!(
        !painted,
        "{} cells painted beside the trench",
        selection.cells.len()
    );
}

// A building placed beside a tunnel's approach finds no road to front: the trench is no
// frontage (before, the farm attached to the approach's trench wall).
#[test]
fn no_building_fronts_a_tunnel_approach() {
    let mut core = trench_core(false);
    let id = register_test_asset(&mut core.allocator, "trench_test", ZoneType::Industrial);
    let mut manifest = core.allocator.registry.get(&id).unwrap().manifest.clone();
    let building = manifest.building.as_mut().unwrap();
    building.placement_mode = PlacementMode::Explicit;
    building.economy_profile = Some("grain_farm_basic".into());
    building.field = Some(crate::assets::asset::BuildingFieldData {
        resource: "grain".into(),
        area_mode: "player_polygon".into(),
    });
    core.allocator
        .registry
        .register("test", manifest, String::new());
    // Beside the trench, and the same farm beside the flat road the tunnel's stroke would
    // continue, which it fronts.
    let placed = core.place_industry_building_internal(&id, -88.0, 16.0);
    assert!(placed.is_err(), "placed beside the trench: {placed:?}");
    let mut flat = trench_core(true);
    let id = register_test_asset(&mut flat.allocator, "trench_test", ZoneType::Industrial);
    let mut manifest = flat.allocator.registry.get(&id).unwrap().manifest.clone();
    let building = manifest.building.as_mut().unwrap();
    building.placement_mode = PlacementMode::Explicit;
    building.economy_profile = Some("grain_farm_basic".into());
    building.field = Some(crate::assets::asset::BuildingFieldData {
        resource: "grain".into(),
        area_mode: "player_polygon".into(),
    });
    flat.allocator
        .registry
        .register("test", manifest, String::new());
    let placed = flat.place_industry_building_internal(&id, -114.0, 16.0);
    assert!(placed.is_ok(), "beside the grounded road: {placed:?}");
}
