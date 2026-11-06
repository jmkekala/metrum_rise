// SPDX-License-Identifier: GPL-2.0-only

//! `PARK-02` parked publication through the real snapshot path: the buffers two snapshots take
//! turns on, and the parked lists across real edits, undo and a load.

use super::*;

// Two snapshots taking turns as the sim thread's do, a custody change on every frame.
#[test]
fn two_snapshots_taking_turns_reuse_a_chunks_buffers() {
    for renderer in [true, false] {
        let mut c = core();
        let id = register_lot(&mut c);
        c.place_service_building_internal(&id, 0.0, -30.0).unwrap();
        c.publish_pending_building_site_changes();
        c.parking.fill_test_fleet();
        let car = c.parking.first_parked_car_for_test();
        let (mut shared, mut recycled) = (RenderSnapshot::default(), RenderSnapshot::default());
        let mut warm = 0;
        for frame in 0..200 {
            c.parking.set_moving(car, frame % 2 == 0);
            let built = c.build_snapshot_reusing(std::mem::take(&mut recycled));
            recycled = std::mem::replace(&mut shared, built);
            if renderer {
                shared.parked.acknowledge(shared.parked.serial);
            }
            if frame == 9 {
                warm = c.parking.buffer_creations();
            }
        }
        assert!(warm > 0);
        assert_eq!(c.parking.buffer_creations(), warm, "renderer {renderer}");
    }
}

fn shown(c: &mut SimCore) -> usize {
    c.build_snapshot();
    c.parking.check_parked_lists().unwrap();
    c.parking.published_poses().len()
}

#[test]
fn the_parked_lists_follow_a_split_a_load_a_bulldoze_and_their_undo() {
    let mut c = core();
    let id = register_lot(&mut c);
    c.place_service_building_internal(&id, -60.0, -30.0)
        .unwrap();
    c.publish_pending_building_site_changes();
    c.parking.fill_test_fleet();
    assert_eq!(shown(&mut c), LOT_BAYS);
    commit(&mut c, (30.0, 0.0), (30.0, 80.0));
    c.publish_pending_building_site_changes();
    assert_eq!(shown(&mut c), LOT_BAYS);
    assert!(c.undo_action_internal());
    c.publish_pending_building_site_changes();
    assert_eq!(shown(&mut c), LOT_BAYS);
    let path = temp_save_path("parked_lists");
    c.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    c.load_game_internal(path.to_str().unwrap()).unwrap();
    assert_eq!(shown(&mut c), LOT_BAYS);
    assert!(c.bulldoze_building(0));
    c.publish_pending_building_site_changes();
    assert_eq!(shown(&mut c), 0);
    assert!(c.undo_action_internal());
    c.publish_pending_building_site_changes();
    assert_eq!(
        shown(&mut c),
        0,
        "restored supply waits for driven arrivals"
    );
    std::fs::remove_file(path).unwrap();
}
