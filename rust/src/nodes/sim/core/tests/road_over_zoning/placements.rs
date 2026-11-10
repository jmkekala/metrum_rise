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

    // Commits the placement: its building's index, or its owner's after it (the take's
    // demolitions may move the owner).
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
                .map(|summary| summary.building_idx),
            Self::Area | Self::AreaResize => core
                .commit_extractor_polygon_internal(owner, polygon)
                .map(|summary| summary.building_idx),
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
                .preview
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
        .preview
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
            .preview
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

impl Placement {
    // The hover the placement's tool shows before its commit (`ZONE-11`, round RZ11b): the
    // building's preview at the commit's pointer, or the field's or area's check; the take it
    // shows, or its refusal.
    fn hover(
        self,
        core: &mut SimCore,
        assets: &Assets,
        owner: usize,
        polygon: &[Vector2],
    ) -> Result<crate::nodes::sim::core::RoadLandTake, String> {
        let hover = match self {
            Self::Service => {
                core.get_service_building_placement_preview_internal(&assets.lot, 0.0, -30.0)
            }
            Self::Industry => {
                core.get_industry_building_placement_preview_internal(&assets.farm, -30.0, -8.0)
            }
            _ => {
                return core
                    .preview_work_area_internal(owner, polygon)
                    .map(|preview| preview.take);
            }
        };
        match hover {
            Ok(hover) => match hover.preview.rejection {
                None => Ok(hover.take),
                Some(rejection) => {
                    assert!(hover.take.is_empty(), "a refused hover shows no take");
                    Err(format!("{rejection:?}"))
                }
            },
            Err(rejection) => Err(format!("{rejection:?}")),
        }
    }
}

// Round RZ11b, per placement: the hover shows exactly what its commit takes. The buildings it
// names are the ones the commit demolishes (by handle), the lots it names the ones removed, the
// painted cells it names the ones erased, and nothing else changes.
#[test]
fn each_hover_shows_what_its_placement_takes() {
    for placement in PLACEMENTS {
        let (mut core, houses, assets) = street();
        let (owner, polygon) = placement
            .prepare(&mut core, &assets, &houses)
            .unwrap_or((usize::MAX, Vec::new()));
        core.prepare_cell_lots_internal();
        let take = placement
            .hover(&mut core, &assets, owner, &polygon)
            .unwrap_or_else(|error| panic!("{placement:?}: {error}"));
        assert!(
            !take.buildings.is_empty(),
            "{placement:?} shows a demolition"
        );
        let handles: Vec<_> = take
            .buildings
            .iter()
            .map(|&building| core.allocator.building_handles.handle(building))
            .collect();
        let others: Vec<_> = (0..core.allocator.buildings.len())
            .filter(|building| !take.buildings.contains(building))
            .map(|building| core.allocator.building_handles.handle(building))
            .collect();
        let painted_before = painted(&core);
        let owner_handle =
            (owner != usize::MAX).then(|| core.allocator.building_handles.handle(owner));
        let placed = placement
            .commit(&mut core, &assets, owner, polygon)
            .unwrap_or_else(|error| panic!("{placement:?}: {error}"));
        if let Some(owner_handle) = owner_handle {
            assert_eq!(
                core.allocator.building_handles.resolve(owner_handle),
                Some(placed),
                "{placement:?}: the commit names its owner where the take left it"
            );
        }
        for handle in &handles {
            assert!(
                core.allocator.building_handles.resolve(*handle).is_none(),
                "{placement:?}: a shown building is demolished"
            );
        }
        for handle in &others {
            assert!(
                core.allocator.building_handles.resolve(*handle).is_some(),
                "{placement:?}: a building not shown stands"
            );
        }
        for lot in &take.lots {
            assert!(
                core.zoning.parcel_by_raw_id(*lot).is_none(),
                "{placement:?}: the shown lot {lot} is removed"
            );
        }
        let painted_after = painted(&core);
        let erased: Vec<_> = painted_before
            .iter()
            .filter(|cell| !painted_after.contains(cell))
            .map(|&(key, _)| key)
            .collect();
        let shown: Vec<_> = take
            .cells
            .iter()
            .filter(|&&(_, paint)| paint != 0)
            .map(|&(key, _)| key)
            .collect();
        assert_eq!(
            erased, shown,
            "{placement:?}: the shown cells are the erased ones"
        );
    }
}

// Round RZ11b: a hover over plotted land shows the commit's refusal and no take, for each
// placement kind: a building over a placed building's lot, a field over another field, an
// extraction area over a placed building's lot.
#[test]
fn a_refused_hover_shows_the_commits_refusal() {
    let (mut core, _, assets) = street();
    let placed = core
        .place_service_building_internal(&assets.lot, 0.0, 30.0)
        .unwrap();
    let (_, _, lot_z0, lot_z1) = bounds(&core.allocator.building_sites[placed].lot_footprint_world);
    let farm = core
        .place_industry_building_internal(&assets.farm, 40.0, 8.0)
        .unwrap();
    let other = core
        .place_industry_building_internal(&assets.farm, 120.0, 8.0)
        .unwrap();
    core.commit_field_polygon_internal(other, rectangle(85.0, 108.0, lot_z0, lot_z0 + 2_000.0))
        .unwrap();
    let mine = core
        .place_industry_building_internal(&assets.mine, -40.0, 8.0)
        .unwrap();
    let strip = |x0, x1| rectangle(x0, x1, lot_z0 + 1.0, lot_z1 - 1.0);
    let state = refused_state(&core);

    let hover = core
        .get_service_building_placement_preview_internal(&assets.lot, 0.0, 30.0)
        .unwrap();
    assert_eq!(
        hover.preview.rejection,
        Some(ExplicitServicePlacementRejection::SiteOverlap)
    );
    assert!(hover.take.is_empty());
    assert_eq!(
        core.place_service_building_internal(&assets.lot, 0.0, 30.0),
        Err(ExplicitServicePlacementRejection::SiteOverlap)
    );

    for (owner, polygon, reason) in [
        (farm, strip(52.0, 95.0), "field overlaps another field"),
        (
            mine,
            strip(-32.0, 20.0),
            "extraction area overlaps a placed building's lot",
        ),
    ] {
        assert_eq!(
            core.preview_work_area_internal(owner, &polygon)
                .err()
                .as_deref(),
            Some(reason)
        );
        let committed = if owner == farm {
            core.commit_field_polygon_internal(owner, polygon)
                .map(|_| ())
        } else {
            core.commit_extractor_polygon_internal(owner, polygon)
                .map(|_| ())
        };
        assert_eq!(committed.err().as_deref(), Some(reason));
    }
    assert_eq!(refused_state(&core), state);
}

// `PARK-01` with `ZONE-11`: a parking lot over the houses on a street that carries no cars is
// refused for its frontage before its land is judged, so its hover shows no take and its commit
// takes nothing (the same lot on the car street takes the houses: `Placement::Service`).
#[test]
fn a_lot_refused_for_a_car_free_frontage_takes_no_land() {
    use crate::simulation::network::types::TransitFlags;
    let (mut core, houses, assets) = street();
    let edge = core.allocator.buildings[houses[0]].edge_idx;
    core.region_graph.edge_mut(edge).allowed_types &= !TransitFlags::CAR;
    let state = refused_state(&core);
    let hover = core
        .get_service_building_placement_preview_internal(&assets.lot, 0.0, -30.0)
        .unwrap();
    assert_eq!(
        hover.preview.rejection,
        Some(ExplicitServicePlacementRejection::NoCarAccess)
    );
    assert!(hover.take.is_empty());
    assert_eq!(
        core.place_service_building_internal(&assets.lot, 0.0, -30.0),
        Err(ExplicitServicePlacementRejection::NoCarAccess)
    );
    assert_eq!(refused_state(&core), state);
}

// Round RZ11b, hover locality: a hover on a stale site index readies only the index's site part
// (`prepare_site_query_chunks`), so the site queries read the chunk lists, while the lists that
// carry the simulation's order (zone, vacancy, admission) and the stale flag stay for the
// index's next reader to rebuild where it would have: whether or when a hover ran does not
// change the simulation. The hover answers as one on a prepared index does.
#[test]
fn a_hover_on_a_stale_index_leaves_the_simulation_order() {
    let (mut core, houses, assets) = street();
    let z = front(&core, &houses);
    let field = rectangle(-99.0, -15.0, z - 500.0, z);
    let order = |core: &SimCore| {
        let allocator = &core.allocator;
        (
            allocator.zone_index.clone(),
            allocator.vacancy_index.clone(),
            allocator.vacancy_pos.clone(),
            allocator.dirty_index,
        )
    };
    // A farm west of the houses, for the field's check, and farms east of them on the far curb:
    // their farmhouses are empty homes. A claim swap-removes one from its vacancy list, out of
    // building order, which the index's next rebuild restores.
    let farm = core
        .place_industry_building_internal(&assets.farm, -110.0, -8.0)
        .unwrap();
    let farm = core.allocator.building_handles.handle(farm);
    let homes = [30.0, 60.0, 90.0, 120.0].map(|x| {
        let home = core
            .place_industry_building_internal(&assets.farm, x, 8.0)
            .unwrap();
        core.allocator.building_handles.handle(home)
    });
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    for (round, home) in [homes[0], homes[1]].into_iter().enumerate() {
        let home = core.allocator.building_handles.resolve(home).unwrap();
        core.allocator.claim_vacancy(home);
        core.allocator.mark_index_stale();
        assert!(!core.allocator.site_chunks_ready(), "round {round}");
        let stale = order(&core);
        let admission = core.allocator.admission_order.clone();
        let mut rebuilt = core.allocator.clone();
        rebuilt.rebuild_zone_index();
        assert_ne!(
            (&rebuilt.vacancy_index, &rebuilt.vacancy_pos),
            (&stale.1, &stale.2),
            "round {round}: a rebuild now would change the vacancy lists"
        );

        let take = if round == 0 {
            let hover = core
                .get_service_building_placement_preview_internal(&assets.lot, 0.0, -30.0)
                .unwrap();
            assert!(hover.preview.valid, "{:?}", hover.preview.rejection);
            hover.take
        } else {
            let owner = core.allocator.building_handles.resolve(farm).unwrap();
            core.preview_work_area_internal(owner, &field).unwrap().take
        };
        assert!(!take.buildings.is_empty(), "round {round}");
        assert_eq!(
            order(&core),
            stale,
            "round {round}: the hover left the order's lists"
        );
        assert!(
            core.allocator.admission_order.same_as(&admission),
            "round {round}"
        );
        assert!(
            core.allocator.site_chunks_ready(),
            "round {round}: the site queries read the chunk lists"
        );

        // The same answer on a prepared index.
        let mut prepared = rebuilt;
        prepared.prepare_building_site_query_index(core.config.zone_cell_m);
        std::mem::swap(&mut core.allocator, &mut prepared);
        let reference = if round == 0 {
            core.get_service_building_placement_preview_internal(&assets.lot, 0.0, -30.0)
                .unwrap()
                .take
        } else {
            let owner = core.allocator.building_handles.resolve(farm).unwrap();
            core.preview_work_area_internal(owner, &field).unwrap().take
        };
        std::mem::swap(&mut core.allocator, &mut prepared);
        assert_eq!(
            (&take.cells, &take.lots, &take.buildings),
            (&reference.cells, &reference.lots, &reference.buildings),
            "round {round}"
        );
    }
}

// Round RZ11b, HANDOFF-RZ11a's open question: a placement on sloped ground over houses does not
// read their terrain pads. The support solve samples the source terrain and the roads, and a
// site's pad never writes the source terrain (the graded terrain derives pads from the sites);
// the taken houses' heights are skipped (`a_demolished_house_does_not_refuse_the_site_height`).
// So the lot placed over the houses stands exactly as the same lot on the same slope with no
// house ever built there, and its hover shows that pose.
#[test]
fn a_placement_on_a_slope_does_not_read_the_demolished_pads() {
    // 2.5 % across the street, rising to +z: the houses' lots fall away from their pads.
    let slope = |core: &mut SimCore| {
        core.slope_terrain_internal(
            Vector2::ZERO,
            256.0,
            Vector2::new(0.0, -128.0),
            0.0,
            Vector2::new(0.0, 128.0),
            6.4,
            1.0,
        )
    };
    let (mut built, _) = street_with_houses_on(slope);
    let lot = register_lot(&mut built);
    let hover = built
        .get_service_building_placement_preview_internal(&lot, 0.0, -30.0)
        .unwrap();
    assert!(hover.preview.valid, "{:?}", hover.preview.rejection);
    assert!(
        !hover.take.buildings.is_empty(),
        "the lot stands over houses"
    );
    let ground = |core: &SimCore, x: f32, z: f32| {
        core.heightmap.sample_height_world(x, z) * crate::config::HEIGHT_SCALE
    };
    for &house in &hover.take.buildings {
        let building = &built.allocator.buildings[house];
        let below =
            building.support_height_m - ground(&built, building.center_x, building.center_y);
        assert!(
            below > 0.2,
            "house {house} stands on a pad {below} m above its ground"
        );
    }
    let corners = hover.preview.corners;
    let placed = built
        .place_service_building_internal(&lot, 0.0, -30.0)
        .unwrap();

    let (mut bare, _) = street_on(slope);
    let bare_lot = register_lot(&mut bare);
    let bare_placed = bare
        .place_service_building_internal(&bare_lot, 0.0, -30.0)
        .unwrap();
    for corner in corners {
        assert_eq!(
            ground(&built, corner.x, corner.y).to_bits(),
            ground(&bare, corner.x, corner.y).to_bits(),
            "the houses left no mark in the source terrain"
        );
    }
    let pose = |core: &SimCore, building: usize| {
        let b = &core.allocator.buildings[building];
        let site = &core.allocator.building_sites[building];
        (
            b.support_height_m.to_bits(),
            b.center_x.to_bits(),
            b.center_y.to_bits(),
            site.footprint_world
                .iter()
                .map(|p| [p.x.to_bits(), p.y.to_bits()])
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        pose(&built, placed),
        pose(&bare, bare_placed),
        "the lot over the demolished houses stands as on bare ground"
    );
    assert_eq!(
        hover.preview.support_height_m.to_bits(),
        built.allocator.buildings[placed].support_height_m.to_bits(),
        "the hover showed the placed height"
    );
}

// The Godot placement demolition test's fixture (`godot/tests/placement_demolition_test.gd`,
// PLAN-RZ 11.6 test 6): the houses' street, saved to `METRUM_PLACEMENT_DEMOLITION_FIXTURE`, and
// beside it, in `<save>.json`, the houses' front line, the bundled parking lot's pointer over
// houses with what its hover here takes, and the farm's and mine's pointers west and east of the
// houses (the game's farm and mine are posed there by the test itself).
#[test]
#[ignore = "writes the Godot placement demolition test's fixture"]
fn record_placement_demolition_fixture() {
    let path = std::env::var("METRUM_PLACEMENT_DEMOLITION_FIXTURE")
        .expect("METRUM_PLACEMENT_DEMOLITION_FIXTURE: path of the fixture save to write");
    let (mut core, houses) = street_with_houses();
    let lot = register_lot(&mut core);
    let hover = core
        .get_service_building_placement_preview_internal(&lot, 0.0, -30.0)
        .unwrap();
    assert!(hover.preview.valid && !hover.take.buildings.is_empty());
    let front_z = front(&core, &houses);
    let house_x: Vec<f32> = houses
        .iter()
        .map(|&house| core.allocator.buildings[house].center_x)
        .collect();
    core.save_game_settled_at(&path, None, 0).unwrap();
    let spec = serde_json::json!({
        "lot_asset": lot,
        "lot_pointer": [0.0, -30.0],
        "lot_demolishes": hover.take.buildings.len(),
        "front_z": front_z,
        "house_x": house_x,
        "farm_pointer": [-110.0, -8.0],
        "mine_pointer": [110.0, -8.0],
    });
    std::fs::write(format!("{path}.json"), spec.to_string()).unwrap();
    eprintln!("PLACEMENT_DEMOLITION_FIXTURE {spec}");
}
