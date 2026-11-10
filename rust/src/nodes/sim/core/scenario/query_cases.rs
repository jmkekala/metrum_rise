// SPDX-License-Identifier: GPL-2.0-only

//! Godot-facing queries leave the simulation's order (`ALLOC-05`): the renderers, pointer
//! raycasts, overlays and inspectors ask the core between frames, whenever Godot's frame asks,
//! so a city played with them runs and records exactly as the same city stepped headless.

use super::save_cases::{differing_tables, save};
use super::*;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;

// Days of play compared frame by frame: the town builds, completes, admits and levels up.
const DAYS: u32 = 3;

// What the queries below would read the city's state through: the zone index's order-carrying
// lists and their stale flag, and the admission order derived from them.
type Order = (bool, [Vec<usize>; 3], [Vec<usize>; 3], Vec<usize>);

fn order(core: &SimCore) -> Order {
    let allocator = &core.allocator;
    (
        allocator.vacancy_index_stale(),
        allocator.zone_index.clone(),
        allocator.vacancy_index.clone(),
        allocator.vacancy_pos.clone(),
    )
}

// The city's products at a day's close: its record, every building (asset, place, level,
// occupancy, construction) and every household's home and size.
type Products = (
    super::GrowthDayRecord,
    Vec<(String, u32, u32, u8, u32, bool)>,
    Vec<(usize, u16)>,
);

fn products(scenario: &mut GrowthScenario) -> Products {
    let record = scenario.record();
    let core = &scenario.core;
    let buildings = core
        .allocator
        .buildings
        .iter()
        .map(|b| {
            (
                b.asset_id.clone(),
                b.center_x.to_bits(),
                b.center_y.to_bits(),
                b.level,
                b.occupancy,
                b.is_under_construction(),
            )
        })
        .collect();
    let households = core
        .households
        .households
        .iter()
        .map(|h| (h.home_building_id, h.member_count))
        .collect();
    (record, buildings, households)
}

// What Godot asks between frames, each through the preparation its bridge makes: the render
// snapshot the simulation thread builds after every frame and edit, the pointer's raycast and
// surface height (`hover_world_surface`, `intersect_world_surface`,
// `get_world_surface_height`), the graded tiles' site snapshot, the building inspector's pick
// (`get_building_info_at`) and the site frame, vegetation, water and agent inspection queries'
// preparation, all over the first building.
fn ask_godot_queries(core: &mut SimCore) {
    core.build_snapshot();
    let at = core
        .allocator
        .buildings
        .first()
        .map_or(Vector2::ZERO, |b| Vector2::new(b.center_x, b.center_y));
    core.prepare_site_queries();
    core.intersect_world_surface_internal(
        Vector3::new(at.x, 400.0, at.y),
        Vector3::new(0.0, -1.0, 0.0),
    );
    core.get_world_surface_height_internal(at);
    let rect = [at.x - 64.0, at.y - 64.0, at.x + 64.0, at.y + 64.0].map(f64::from);
    assert!(
        core.allocator.graded_site_snapshot(vec![rect]).is_some(),
        "the graded tiles read the sites a query readied"
    );
    let cell_m = core.config.zone_cell_m;
    core.allocator.nearest_building_idx_at(at.x, at.y, cell_m);
}

// The player's edit `k` between two frames, the same in both towns: a parking lot placed on the
// main street west of the town (a placement leaves the index stale for its next reader), or the
// first occupied home bulldozed (its households are rehoused). Whether it applied.
fn edit(core: &mut SimCore, k: u32) -> bool {
    if k.is_multiple_of(2) {
        let x = -320.0 - 35.0 * (k / 2) as f32;
        core.place_service_building_internal(SMALL_LOT, x, 8.0)
            .is_ok()
    } else {
        core.allocator
            .buildings
            .iter()
            .position(|b| b.zone_type == ZoneType::Residential && b.occupancy > 0)
            .is_some_and(|home| core.bulldoze_building(home))
    }
}

const SMALL_LOT: &str = "parking:building.parking.small_lot";

// The same town played twice in lockstep, with the same edits every other hour; Godot's queries
// are asked of one after every frame and every edit, as the game asks them. The order-carrying
// lists after each frame and edit, the products at each day's close and a save made right after
// an edit are equal. On `da5ede14` the queries rebuilt a stale index whenever Godot asked.
#[test]
fn godot_queries_leave_the_days_and_their_records() {
    let mut headless = GrowthScenario::starter_town();
    let mut played = GrowthScenario::starter_town();
    let (mut frames, mut days, mut edits, mut applied, mut stale_edits) = (0_u64, 0, 0, 0, 0);
    let mut first_order_difference = None;
    let mut first_product_difference = None;
    let mut hour = headless.core.time.absolute_slot() / SLOTS_PER_HOUR;
    let mut compare_order = |headless: &SimCore, played: &SimCore, frames: u64| {
        if first_order_difference.is_none() && order(headless) != order(played) {
            first_order_difference = Some(frames);
        }
    };
    while days < DAYS {
        frames += 1;
        let daily = headless.core.step_frame(FRAME_DT_S).daily_ticks > 0;
        assert_eq!(played.core.step_frame(FRAME_DT_S).daily_ticks > 0, daily);
        ask_godot_queries(&mut played.core);
        compare_order(&headless.core, &played.core, frames);
        let now = headless.core.time.absolute_slot() / SLOTS_PER_HOUR;
        if now != hour && now.is_multiple_of(2) {
            let done = edit(&mut headless.core, edits);
            assert_eq!(edit(&mut played.core, edits), done, "edit {edits}");
            edits += 1;
            applied += u32::from(done);
            stale_edits += u32::from(headless.core.allocator.vacancy_index_stale());
            ask_godot_queries(&mut played.core);
            compare_order(&headless.core, &played.core, frames);
        }
        hour = now;
        if daily {
            days += 1;
            if first_product_difference.is_none()
                && products(&mut headless) != products(&mut played)
            {
                first_product_difference = Some(days);
            }
        }
    }
    assert!(
        applied > 0 && stale_edits > 0,
        "{applied} of {edits} edits applied, {stale_edits} left the index stale"
    );
    // A save right after an edit that leaves the index stale for its next reader.
    let mut stale_at_save = false;
    for k in edits..edits + 16 {
        let done = edit(&mut headless.core, k);
        assert_eq!(edit(&mut played.core, k), done);
        ask_godot_queries(&mut played.core);
        if done && headless.core.allocator.vacancy_index_stale() {
            stale_at_save = true;
            break;
        }
    }
    assert!(stale_at_save, "an edit left the index stale");
    compare_order(&headless.core, &played.core, frames);
    let path = |name: &str| {
        std::env::temp_dir().join(format!(
            "metrum-alloc05-{name}-{}.sqlite",
            std::process::id()
        ))
    };
    let (first, second) = (path("headless"), path("played"));
    save(&mut headless.core, &first);
    save(&mut played.core, &second);
    let same_save = std::fs::read(&first).unwrap() == std::fs::read(&second).unwrap();
    let differing = if same_save {
        Vec::new()
    } else {
        differing_tables(&first, &second)
    };
    for path in [first, second] {
        std::fs::remove_file(path).unwrap();
    }
    let run = format!(
        "{frames} frames, {edits} edits ({applied} applied, {stale_edits} left the index stale)"
    );
    assert_eq!(
        first_order_difference,
        None,
        "frame whose end or edit first left other order lists, {run}; products equal from \
         the first day: {}, saves equal: {same_save}",
        first_product_difference.is_none()
    );
    assert_eq!(
        first_product_difference, None,
        "day of the first product difference, {run}"
    );
    assert!(
        same_save,
        "the played town's save: tables differing in storage order: {differing:?}"
    );
}

// A reader outside the simulation's order, asked at a point.
type Reader = fn(&mut SimCore, DVec2);

// Each reader outside the simulation's order, asked of a stale index whose rebuild would reorder
// the vacancy lists (a desertion swap-removed a home out of building order): the zone and
// vacancy lists, their positions, the admission order and the stale flag stay for the index's
// next reader, and the site queries are ready. The cell overlay's chunk (the simulation
// thread's request and its stale chunks) and a cell gesture's preview are asked here; the
// other readers through `ask_godot_queries`.
#[test]
fn each_godot_reader_leaves_a_stale_index_to_its_next_reader() {
    let mut town = GrowthScenario::starter_town();
    town.run_days(2);
    let core = &mut town.core;
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let homes = core.allocator.vacancy_index[0].clone();
    assert!(homes.len() > 2, "vacant homes: {homes:?}");
    core.allocator.mark_building_deserted(homes[0]);
    core.allocator.mark_index_stale();
    let stale = order(core);
    let admission = core.allocator.admission_order.clone();
    let mut rebuilt = core.allocator.clone();
    rebuilt.rebuild_zone_index();
    assert_ne!(
        (&rebuilt.vacancy_index, &rebuilt.vacancy_pos),
        (&stale.2, &stale.3),
        "a rebuild now would reorder the vacancy lists"
    );
    let at = DVec2::new(-200.0, 60.0);
    core.cell_overlay_visible = true;
    let readers: [(&str, Reader); 4] = [
        ("Godot's queries", |core, _| ask_godot_queries(core)),
        ("the overlay's requested chunk", |core, _| {
            let generation = core.terrain_payload_global_generation;
            assert!(core.prepare_requested_cell_chunk((0, 0), generation));
        }),
        ("the overlay's stale chunks", |core, _| {
            core.prepare_stale_cell_chunks_internal()
        }),
        ("a cell gesture's preview", |core, at| {
            core.preview_cell_selection_internal(ZONE_BRUSH, &[at, at + DVec2::new(0.0, 40.0)]);
        }),
    ];
    for (reader, ask) in readers {
        ask(core, at);
        assert!(order(core) == stale, "{reader} left the order's lists");
        assert!(
            core.allocator.admission_order.same_as(&admission),
            "{reader} left the admission order"
        );
        assert!(
            core.allocator.building_site_query_index_ready(),
            "{reader}: the site queries are ready"
        );
    }
}
