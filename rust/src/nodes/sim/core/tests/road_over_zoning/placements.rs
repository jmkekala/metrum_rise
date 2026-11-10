// SPDX-License-Identifier: GPL-2.0-only

//! Plotted placements over zoned land (`ZONE-11`, round RZ11a): an explicit building (a service
//! lot, a farm), a field (new or resized) and an extraction area (new or resized) take the zoned
//! land under their lot or polygon as a road does: the growable buildings on the lots it covers
//! are demolished, those lots removed and only the covered cells' paint erased, with no undo
//! record (decision D-undo); plotted land (a field, an extraction area, a placed building's lot)
//! refuses each of them with nothing changed, a farm's or mine's own field, area and lot
//! excepted. An extraction area blocks cells and parcels as a field does (decision
//! D-extraction-blocks).

use super::*;
use crate::nodes::sim::core::tests::parking::register_lot;
use crate::simulation::buildings::allocator::ExplicitServicePlacementRejection;

// The three explicit assets: the parking lot (a service building), a farm and a mine (industry
// buildings with a field and an extraction area), all on 2 x 2 cell lots.
struct Assets {
    lot: String,
    farm: String,
    mine: String,
}

fn register_assets(core: &mut SimCore) -> Assets {
    use crate::assets::asset::{BuildingExtractorData, BuildingFieldData};
    let lot = register_lot(core);
    let farm = register_test_asset(&mut core.allocator, "zone11_farm", ZoneType::Industrial);
    let mut manifest = core.allocator.registry.get(&farm).unwrap().manifest.clone();
    let building = manifest.building.as_mut().unwrap();
    building.placement_mode = PlacementMode::Explicit;
    building.economy_profile = Some("grain_farm_basic".into());
    building.field = Some(BuildingFieldData {
        resource: "grain".into(),
        area_mode: "player_polygon".into(),
    });
    let mut mine = manifest.clone();
    mine.asset_id = "zone11_mine".to_owned();
    let building = mine.building.as_mut().unwrap();
    building.economy_profile = Some("coal_mine_basic".into());
    building.field = None;
    building.extractor = Some(BuildingExtractorData {
        resource: "coal".into(),
        area_mode: "player_polygon".into(),
    });
    core.allocator
        .registry
        .register("test", manifest, String::new());
    core.allocator
        .registry
        .register("test", mine, String::new());
    Assets {
        lot,
        farm,
        mine: "test:zone11_mine".to_owned(),
    }
}

// The houses' street (`street_with_houses`) with the three explicit assets registered.
fn street() -> (SimCore, Vec<usize>, Assets) {
    let (mut core, houses) = street_with_houses();
    let assets = register_assets(&mut core);
    (core, houses, assets)
}

// An axis-aligned rectangle over x0..x1, z0..z1.
fn rectangle(x0: f32, x1: f32, z0: f32, z1: f32) -> Vec<Vector2> {
    vec![
        Vector2::new(x0, z0),
        Vector2::new(x1, z0),
        Vector2::new(x1, z1),
        Vector2::new(x0, z1),
    ]
}

// A polygon's bounds: (min x, max x, min z, max z).
fn bounds(points: &[Vector2]) -> (f32, f32, f32, f32) {
    points.iter().fold(
        (
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ),
        |(x0, x1, z0, z1), p| (x0.min(p.x), x1.max(p.x), z0.min(p.y), z1.max(p.y)),
    )
}

// The houses' lots' front line (world z, negative): the sidewalk's outer edge on their curb.
fn front(core: &SimCore, houses: &[usize]) -> f32 {
    houses
        .iter()
        .map(|&house| bounds(&core.allocator.building_sites[house].lot_footprint_world).3)
        .fold(f32::NEG_INFINITY, f32::max)
}

// Whether a cell's road-contact interior overlaps `polygon` with positive area: the oracle of
// "under the placement".
fn under(core: &SimCore, key: CellKey, polygon: &[Vector2]) -> bool {
    let frame = core.zoning.cells.frame(key.grid).unwrap();
    let interior = road_contact_interior(&frame.corners(key.x, key.y)).map(|p| [p.x, p.y]);
    PolygonFootprint::new(polygon).overlaps_precise_points(&interior)
}

// One placement under test, made on the houses' street (houses on its -z curb, x -60 to 60).
#[derive(Clone, Copy, Debug)]
enum Placement {
    // The parking lot over the houses at x 0 (a service building).
    Service,
    // A farm over the houses at x -30 (an industry building).
    Industry,
    // A farm west of the houses with a new field over the block's west half and behind it.
    Field,
    // That farm's field first behind the block only, then resized over the block's west half.
    FieldResize,
    // A mine east of the houses with a new area over the block's east half.
    Area,
    // That mine's area first beside its own lot only, then resized over the block's east half.
    AreaResize,
}

const PLACEMENTS: [Placement; 6] = [
    Placement::Service,
    Placement::Industry,
    Placement::Field,
    Placement::FieldResize,
    Placement::Area,
    Placement::AreaResize,
];

impl Placement {
    // Places the owner of a field or area (west or east of the houses) and its first polygon
    // when the placement is a resize; returns the owner and the polygon the placement draws.
    fn prepare(
        self,
        core: &mut SimCore,
        assets: &Assets,
        houses: &[usize],
    ) -> Option<(usize, Vec<Vector2>)> {
        let z = front(core, houses);
        // A field covers at least 4 ha: the block west of x -15 and the land 25 m behind the
        // lots, 300 m deep, the second a field of its own.
        let behind = rectangle(-99.0, 99.0, z - 300.0, z - 25.0);
        let field = vec![
            Vector2::new(-99.0, z),
            Vector2::new(-99.0, z - 300.0),
            Vector2::new(99.0, z - 300.0),
            Vector2::new(99.0, z - 25.0),
            Vector2::new(-15.0, z - 25.0),
            Vector2::new(-15.0, z),
        ];
        let (asset, x, first, polygon) = match self {
            Self::Service | Self::Industry => return None,
            Self::Field => (&assets.farm, -110.0, None, field),
            Self::FieldResize => (&assets.farm, -110.0, Some(behind), field),
            Self::Area => (
                &assets.mine,
                110.0,
                None,
                rectangle(15.0, 99.0, z - 40.0, z),
            ),
            Self::AreaResize => (
                &assets.mine,
                110.0,
                Some(rectangle(80.0, 99.0, z - 40.0, z)),
                rectangle(15.0, 99.0, z - 40.0, z),
            ),
        };
        let owner = core
            .place_industry_building_internal(asset, x, -8.0)
            .expect("the owner stands away from the houses");
        if let Some(first) = first {
            self.commit(core, assets, owner, first)
                .expect("the first polygon holds no house");
        }
        Some((owner, polygon))
    }

    // Commits the placement: its building's index, or its owner's.
    fn commit(
        self,
        core: &mut SimCore,
        assets: &Assets,
        owner: usize,
        polygon: Vec<Vector2>,
    ) -> Result<usize, String> {
        let error = |rejection| format!("{rejection:?}");
        match self {
            Self::Service => core
                .place_service_building_internal(&assets.lot, 0.0, -30.0)
                .map_err(error),
            Self::Industry => core
                .place_industry_building_internal(&assets.farm, -30.0, -8.0)
                .map_err(error),
            Self::Field | Self::FieldResize => core
                .commit_field_polygon_internal(owner, polygon)
                .map(|_| owner),
            Self::Area | Self::AreaResize => core
                .commit_extractor_polygon_internal(owner, polygon)
                .map(|_| owner),
        }
    }
}

// A placement made on a fresh street, with the footprint it covers (the lot of a building, the
// polygon of a field or area) and the houses' lots and the land as they were before it.
struct Placed {
    core: SimCore,
    footprint: Vec<Vector2>,
    lots: Vec<(u64, Vec<CellKey>)>,
    painted_before: Vec<(CellKey, u16)>,
    buildings_before: usize,
    agents_before: usize,
    undo_depth: usize,
}

fn place(placement: Placement) -> Placed {
    let (mut core, houses, assets) = street();
    let prepared = placement.prepare(&mut core, &assets, &houses);
    core.prepare_cell_lots_internal();
    let lots = lots_of(&core, &houses);
    let painted_before = painted(&core);
    let buildings_before = core.allocator.buildings.len();
    let agents_before = core.agents.agents.len();
    let undo_depth = core.undo_stack.len();
    let (owner, polygon) = prepared.clone().unwrap_or((usize::MAX, Vec::new()));
    let placed = placement
        .commit(&mut core, &assets, owner, polygon)
        .unwrap_or_else(|error| panic!("{placement:?}: {error}"));
    let footprint = match prepared {
        Some((_, polygon)) => polygon,
        None => core.allocator.building_sites[placed]
            .lot_footprint_world
            .to_vec(),
    };
    Placed {
        core,
        footprint,
        lots,
        painted_before,
        buildings_before,
        agents_before,
        undo_depth,
    }
}

// `ZONE-11`, per placement: the lots the footprint covers are taken with their houses (the
// families evacuated, every reference consistent), only the covered cells lose their paint, a
// taken lot's other cells stay painted and free, no paint is inherited, no reserved cell is left
// under the footprint, the other houses stand, and no undo record is pushed (decision D-undo).
#[test]
fn each_placement_takes_the_zoned_land_under_it() {
    // Cells of taken lots outside the footprints: a lot aligned with the footprint has none.
    let mut leftover = 0;
    for placement in PLACEMENTS {
        let Placed {
            mut core,
            footprint,
            lots,
            painted_before,
            buildings_before,
            agents_before,
            undo_depth,
        } = place(placement);
        let (taken, kept): (Vec<_>, Vec<_>) = lots
            .iter()
            .partition(|(_, cells)| cells.iter().any(|&key| under(&core, key, &footprint)));
        assert!(!taken.is_empty(), "{placement:?} covers a house's lot");
        assert!(!kept.is_empty(), "{placement:?} leaves houses beside it");
        for (id, _) in &taken {
            assert!(
                core.zoning.parcel_by_raw_id(*id).is_none(),
                "{placement:?}: the taken lot {id} is removed"
            );
        }
        for (id, _) in &kept {
            assert!(
                core.zoning
                    .parcel_by_raw_id(*id)
                    .and_then(|parcel| parcel.occupied_building())
                    .is_some(),
                "{placement:?}: the kept lot {id} keeps its house"
            );
        }
        let placed_buildings = usize::from(matches!(
            placement,
            Placement::Service | Placement::Industry
        ));
        assert_eq!(
            core.allocator.buildings.len(),
            buildings_before - taken.len() + placed_buildings,
            "{placement:?}: one house demolished per taken lot"
        );
        assert_eq!(core.agents.agents.len(), agents_before, "{placement:?}");
        assert_references_consistent(&core);
        assert_eq!(
            core.undo_stack.len(),
            undo_depth,
            "{placement:?}: no record"
        );

        let painted_after = painted(&core);
        for &(key, profile) in &painted_before {
            let covered = under(&core, key, &footprint);
            assert_eq!(
                painted_after.contains(&(key, profile)),
                !covered,
                "{placement:?} {key:?}: covered {covered}"
            );
        }
        assert!(
            painted_after
                .iter()
                .all(|cell| painted_before.contains(cell)),
            "{placement:?}: no cell inherits paint"
        );
        for (_, cells) in &taken {
            for &key in cells.iter().filter(|&&key| !under(&core, key, &footprint)) {
                leftover += 1;
                let lot = core.zoning.cells.lot(key).unwrap_or(0);
                assert!(
                    taken.iter().all(|(id, _)| *id != lot),
                    "{placement:?} {key:?}: a taken lot's leftover cell is free"
                );
            }
        }
        core.prepare_cell_lots_internal();
        let mut reserved_under = Vec::new();
        core.zoning
            .cells
            .visit_reserved_footprints_in_bounds(AREA, |key, _| {
                if under(&core, key, &footprint) {
                    reserved_under.push(key);
                }
            });
        assert!(
            reserved_under.is_empty(),
            "{placement:?}: {reserved_under:?}"
        );
        assert_references_consistent(&core);
    }
    assert!(leftover > 0, "a placement leaves part of a taken lot");
}

// The field and area edits plan their lots themselves when they erased paint (decision
// D-lot-plan): the erase's paint is drained before the commit returns, so a second erasing edit
// before any frame does not find the first one's, and no lot region is left queued.
#[test]
fn a_work_area_that_erased_paint_plans_its_lots() {
    for placement in [Placement::Field, Placement::Area] {
        let Placed { core, .. } = place(placement);
        assert!(
            !core.zoning.has_road_paint_sources(),
            "{placement:?}: the edit's own plan took the erased paint"
        );
        assert!(
            !core.zoning.has_dirty_cell_lots(),
            "{placement:?}: the lots under and beside the polygon are planned"
        );
    }
}

// What a refused placement must leave as it was: the land, buildings and history
// (`land_state`), the fields and areas.
fn refused_state(core: &SimCore) -> (Vec<u64>, String, String) {
    let polygons = |sites: Vec<(usize, Vec<Vector2>)>| format!("{sites:?}");
    (
        land_state(core),
        polygons(
            core.agriculture
                .sites()
                .iter()
                .map(|site| (site.building_idx, site.polygon_world.clone()))
                .collect(),
        ),
        polygons(
            core.resource_extraction
                .sites()
                .iter()
                .map(|site| (site.building_idx, site.polygon_world.clone()))
                .collect(),
        ),
    )
}

// The plotted land the refusal test draws over, on the street's +z curb (no houses there): a
// placed building's lot, a farm field, a mine's extraction area.
#[derive(Clone, Copy, Debug)]
enum Plotted {
    Lot,
    Field,
    Area,
}

// Plotted land refuses every placement over it, each with its reason, and nothing is written:
// an explicit building over it (its preview too), a field and an extraction area of other
// owners over it.
#[test]
fn plotted_land_refuses_every_placement() {
    for plotted in [Plotted::Lot, Plotted::Field, Plotted::Area] {
        let (mut core, _, assets) = street();
        // The plotted owner at x 0, on the +z curb; its lot spans x -10 to 10.
        let owner = match plotted {
            Plotted::Lot => core.place_service_building_internal(&assets.lot, 0.0, 30.0),
            Plotted::Field => core.place_industry_building_internal(&assets.farm, 0.0, 8.0),
            Plotted::Area => core.place_industry_building_internal(&assets.mine, 0.0, 8.0),
        }
        .unwrap();
        let (_, _, lot_z0, lot_z1) =
            bounds(&core.allocator.building_sites[owner].lot_footprint_world);
        // A field or area along the frontage east of the lot, 4.2 ha.
        let polygon = rectangle(12.0, 40.0, lot_z0, lot_z0 + 1_500.0);
        match plotted {
            Plotted::Lot => {}
            Plotted::Field => {
                core.commit_field_polygon_internal(owner, polygon.clone())
                    .unwrap();
            }
            Plotted::Area => {
                core.commit_extractor_polygon_internal(owner, polygon.clone())
                    .unwrap();
            }
        }
        // The other farm and mine, east and west.
        let farm = core
            .place_industry_building_internal(&assets.farm, 70.0, 8.0)
            .unwrap();
        let mine = core
            .place_industry_building_internal(&assets.mine, -40.0, 8.0)
            .unwrap();
        let state = refused_state(&core);

        // An explicit building over the plotted land.
        let (x, expected) = match plotted {
            Plotted::Lot => (0.0, ExplicitServicePlacementRejection::SiteOverlap),
            Plotted::Field => (26.0, ExplicitServicePlacementRejection::FieldOverlap),
            Plotted::Area => (
                26.0,
                ExplicitServicePlacementRejection::ExtractionAreaOverlap,
            ),
        };
        assert_eq!(
            core.get_service_building_placement_preview_internal(&assets.lot, x, 30.0)
                .unwrap()
                .rejection,
            Some(expected),
            "{plotted:?}"
        );
        assert_eq!(
            core.place_service_building_internal(&assets.lot, x, 30.0),
            Err(expected),
            "{plotted:?}"
        );
        assert_eq!(refused_state(&core), state, "{plotted:?}");

        // The other farm's field and the other mine's area over it, each from its own lot, in a
        // strip inside the lots' depth.
        let strip = |x0, x1| rectangle(x0, x1, lot_z0 + 1.0, lot_z1 - 1.0);
        let (field_reason, area_reason) = match plotted {
            Plotted::Lot => (
                "field overlaps a placed building's lot",
                "extraction area overlaps a placed building's lot",
            ),
            Plotted::Field => (
                "field overlaps another field",
                "extraction area overlaps a farm field",
            ),
            Plotted::Area => (
                "field overlaps a mine's extraction area",
                "extraction area overlaps another mine's extraction area",
            ),
        };
        let field = match plotted {
            Plotted::Lot => strip(5.0, 62.0),
            _ => strip(30.0, 62.0),
        };
        assert_eq!(
            core.validate_field_polygon_internal(farm, &field)
                .unwrap_err(),
            field_reason,
            "{plotted:?}"
        );
        assert_eq!(
            core.commit_field_polygon_internal(farm, field).unwrap_err(),
            field_reason,
            "{plotted:?}"
        );
        assert_eq!(
            core.commit_extractor_polygon_internal(mine, strip(-32.0, 20.0))
                .unwrap_err(),
            area_reason,
            "{plotted:?}"
        );
        assert_eq!(refused_state(&core), state, "{plotted:?}");
    }
}

// The owner rule: a field may cover its own farm's lot and its own old polygon, an area its own
// mine's lot and old area; the farm's and mine's own building footprints still refuse them.
#[test]
fn a_work_area_may_cover_its_owners_lot_and_old_polygon() {
    let (mut core, _, assets) = street();
    for (asset, x, field) in [(&assets.farm, -100.0, true), (&assets.mine, 100.0, false)] {
        // On the +z curb: the lot's back is its largest z.
        let owner = core
            .place_industry_building_internal(asset, x, 8.0)
            .unwrap();
        let site = &core.allocator.building_sites[owner];
        let lot = site.lot_footprint_world.to_vec();
        let (lot_x0, lot_x1, _, lot_back) = bounds(&lot);
        let (_, _, _, building_back) = bounds(&site.footprint_world);
        assert!(
            lot_back > building_back + 1.0,
            "the lot reaches behind the building: lot {lot_back}, building {building_back}"
        );
        // From inside the lot behind the building, 4.5 ha.
        let first = rectangle(lot_x0 - 40.0, lot_x1 + 40.0, building_back + 0.5, 450.0);
        assert!(PolygonFootprint::new(&first).overlaps(&PolygonFootprint::new(&lot)));
        let commit = |core: &mut SimCore, polygon| {
            if field {
                core.commit_field_polygon_internal(owner, polygon)
                    .map(|_| ())
            } else {
                core.commit_extractor_polygon_internal(owner, polygon)
                    .map(|_| ())
            }
        };
        commit(&mut core, first.clone()).expect("over its own lot");
        let wider = rectangle(lot_x0 - 45.0, lot_x1 + 45.0, building_back + 0.5, 500.0);
        commit(&mut core, wider).expect("over its own old polygon");
        let over_building = rectangle(lot_x0 - 40.0, lot_x1 + 40.0, building_back - 1.0, 450.0);
        let label = if field { "field" } else { "extraction area" };
        assert_eq!(
            commit(&mut core, over_building).unwrap_err(),
            format!("{label} overlaps a building site")
        );
    }
}

// Decision D-extraction-blocks: no cell is generated and no parcel accepted inside an area; a
// painted cell standing under an area drawn before areas blocked zoning (an old save) keeps its
// paint, and the next edit of the area takes it.
#[test]
fn an_extraction_area_blocks_cells_and_parcels() {
    let (mut core, houses, assets) = street();
    let mine = core
        .place_industry_building_internal(&assets.mine, 110.0, -8.0)
        .unwrap();
    let z = front(&core, &houses);
    let area = rectangle(70.0, 99.0, z - 40.0, z);
    assert!(core.prepare_cell_chunk_internal((0, 0)));
    let cells_under = |core: &SimCore, polygon: &[Vector2]| {
        let mut cells = Vec::new();
        core.zoning.cells.visit_in_bounds(AREA, |key| {
            if under(core, key, polygon) {
                cells.push(key);
            }
        });
        cells
    };
    assert!(
        !cells_under(&core, &area).is_empty(),
        "cells before the area"
    );
    core.commit_extractor_polygon_internal(mine, area.clone())
        .unwrap();
    assert!(core.prepare_cell_chunk_internal((0, 0)));
    assert_eq!(
        cells_under(&core, &area),
        Vec::new(),
        "no cell inside the area"
    );
    let geometry = core
        .zoning
        .preview_parcel_at(85.0, z - 15.0, 20.0, 20.0, &core.region_graph)
        .unwrap();
    let residential = core
        .zoning
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();
    assert_eq!(
        core.allocator.zoning_site_feasibility(
            &geometry,
            residential,
            &core.zoning,
            &core.region_graph,
            BuildingSiteEnvironment {
                road_surface: &core.transit_network.roadbed,
                terrain: &core.heightmap,
            },
        ),
        Err("extraction_area_overlap")
    );

    // An old save's area over a house's painted cells, written straight into the index as a
    // load does.
    let house = houses
        .iter()
        .copied()
        .find(|&house| core.allocator.buildings[house].center_x > 30.0)
        .expect("a house east of x 30");
    let (x0, x1, _, _) = bounds(&core.allocator.building_sites[house].lot_footprint_world);
    let legacy = rectangle(x0, x1, z - 40.0, z);
    let under_legacy: Vec<_> = painted(&core)
        .into_iter()
        .filter(|&(key, _)| under(&core, key, &legacy))
        .collect();
    assert!(!under_legacy.is_empty());
    let legacy_owner = core.allocator.buildings.len() + 7;
    core.allocator
        .extraction_clearance_mut()
        .set(legacy_owner, &legacy);
    core.zoning.mark_cell_lots_dirty(CellBounds::from_points(
        legacy
            .iter()
            .map(|p| DVec2::new(f64::from(p.x), f64::from(p.y))),
    ));
    core.prepare_cell_lots_internal();
    let painted_after = painted(&core);
    for cell in &under_legacy {
        assert!(painted_after.contains(cell), "{cell:?} keeps its paint");
    }
    core.allocator
        .extraction_clearance_mut()
        .set(legacy_owner, &[]);
    // The mine's area resized over those cells takes them, the house with them.
    let buildings = core.allocator.buildings.len();
    core.commit_extractor_polygon_internal(mine, rectangle(x0, 99.0, z - 40.0, z))
        .unwrap();
    for (key, _) in &under_legacy {
        assert!(
            core.zoning
                .cells
                .profile(*key)
                .is_none_or(|profile| profile == 0),
            "{key:?} is taken"
        );
    }
    assert!(
        core.allocator.buildings.len() < buildings,
        "the house is taken"
    );
    assert_references_consistent(&core);
}

// Determinism: two cities making the same placements end in the same land, cells, buildings and
// polygons, after their lot plans too.
#[test]
fn placements_take_land_deterministically() {
    let run = || {
        PLACEMENTS
            .map(|placement| {
                let Placed { mut core, .. } = place(placement);
                core.prepare_cell_lots_internal();
                (core.zoning.cells.state_digest(), refused_state(&core))
            })
            .to_vec()
    };
    assert_eq!(run(), run());
}

// A placement over houses whose sites stand at another height: the houses under the lot are
// demolished, so their heights constrain nothing and the placement (its preview too) is not
// refused for them.
#[test]
fn a_demolished_house_does_not_refuse_the_site_height() {
    let (mut core, houses, assets) = street();
    let lot = core
        .get_service_building_placement_preview_internal(&assets.lot, 0.0, -30.0)
        .unwrap()
        .corners;
    let lots = lots_of(&core, &houses);
    let under_lot: Vec<_> = houses
        .iter()
        .zip(&lots)
        .filter(|(_, (_, cells))| cells.iter().any(|&key| under(&core, key, &lot)))
        .map(|(&house, _)| house)
        .collect();
    assert!(!under_lot.is_empty());
    for &house in &under_lot {
        core.allocator.building_sites[house].support_height_m += 3.0;
    }
    assert_eq!(
        core.get_service_building_placement_preview_internal(&assets.lot, 0.0, -30.0)
            .unwrap()
            .rejection,
        None
    );
    let before = core.allocator.buildings.len();
    let placed = core
        .place_service_building_internal(&assets.lot, 0.0, -30.0)
        .expect("the houses under the lot are demolished, not neighbours");
    assert_eq!(core.allocator.buildings.len(), before - under_lot.len() + 1);
    assert_eq!(core.allocator.buildings[placed].parcel_id, 0);
}
