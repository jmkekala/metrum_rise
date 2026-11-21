// SPDX-License-Identifier: GPL-3.0-or-later

//! The catalogue of the bundled `vehicles` pack: order, picks, tiers, lamps and ground support.

use super::*;
use crate::assets::scan_pack_dir;

fn bundled() -> VehicleCatalogue {
    let mods = Path::new(env!("CARGO_MANIFEST_DIR")).join("../godot/bootstrap/mods");
    let mut registry = AssetRegistry::new();
    for pack in scan_pack_dir(&mods).packs {
        for (manifest, directory) in pack.assets {
            registry.register(&pack.pack.pack_id, manifest, directory);
        }
    }
    VehicleCatalogue::from_registry(&registry)
}

#[test]
fn the_bundled_pack_gives_the_estate_and_the_truck_in_three_looks() {
    let catalogue = bundled();
    let ids: Vec<_> = catalogue.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["vehicles:vehicle.civil.estate_compact", "vehicles:vehicle.utility.rigid_box_18t"]);
    let looks: Vec<_> = catalogue.looks.iter().map(|l| (l.model, l.variant)).collect();
    assert_eq!(looks, [(0, 0), (1, 0), (1, 1)]);
    assert_eq!(catalogue.models[1].data.variants[1].name, "haulier");
    assert!(!catalogue.models[1].data.variants[1].paint);
    assert_eq!(catalogue.models[0].tier_starts_m, [0.0, 30.0, 100.0, 398.0]);
}

#[test]
fn picks_are_stable_and_spread_over_variants_and_paints() {
    let catalogue = bundled();
    let mut truck_looks = [0; 3];
    let mut paints = [0; MAX_LOOKS];
    for identity in 0..4000u64 {
        let car = catalogue.pick(false, identity).unwrap();
        assert_eq!(car, catalogue.pick(false, identity).unwrap());
        assert_eq!((car.look, car.model), (0, 0));
        assert!(car.blink < 16);
        paints[usize::from(car.paint)] += 1;
        let truck = catalogue.pick(true, identity).unwrap();
        assert_eq!(truck.model, 1);
        truck_looks[usize::from(truck.look)] += 1;
    }
    assert_eq!(truck_looks[0], 0);
    assert!(truck_looks[1] > 1500 && truck_looks[2] > 1500, "{truck_looks:?}");
    let palette = catalogue.models[0].palette.len();
    assert!(paints[..palette].iter().all(|&n| n > 0), "{paints:?}");
    // White, the heaviest paint, is the most common.
    assert_eq!(paints.iter().enumerate().max_by_key(|(_, n)| **n).unwrap().0, 0);
    assert!(VehicleCatalogue::default().pick(false, 1).is_none());
}

#[test]
fn the_word_packs_lamps_paint_and_blink_exactly() {
    let drawn = DrawnVehicle { look: 0, model: 0, paint: 63, blink: 15 };
    let word = drawn.word(0xff) as u32;
    assert_eq!((word & 0xff, (word >> 8) & 63, word >> 14), (0xff, 63, 15));
    assert_eq!(drawn.custom(LAMP_LIGHTS_ON)[2] as u32 & 0xff, 1);
}

#[test]
fn tiers_switch_at_the_asset_starts_and_lamps_sit_in_the_turned_frame() {
    let catalogue = bundled();
    let tier = |model, d: f32| catalogue.tier(model, d * d);
    assert_eq!([tier(0, 0.0), tier(0, 29.9), tier(0, 30.0), tier(0, 99.0), tier(0, 397.0), tier(0, 400.0)], [0, 0, 1, 1, 2, 3]);
    assert_eq!([tier(1, 50.0), tier(1, 60.0), tier(1, 700.0), tier(1, 710.0)], [0, 1, 2, 3]);
    let estate = &catalogue.models[0];
    // The right (-X) low-beam lens, nose at -Z after the half turn; the right tail lamp at +Z.
    let lens = |group| {
        estate.data.lamps.iter().find(|l| l.group == group && l.centre[0] < 0.0).unwrap().centre
    };
    let head = estate.lamp(&[LampGroup::LowBeam], 1.0).unwrap();
    assert_eq!(head, Vector3::from_array(turned(lens(LampGroup::LowBeam))));
    assert!(head.x > 0.0 && head.z < -1.5, "{head:?}");
    let tail = estate.lamp(&[LampGroup::TailBrake, LampGroup::Tail], -1.0).unwrap();
    assert_eq!(tail, Vector3::from_array(turned(lens(LampGroup::TailBrake))));
    assert!(tail.x > 0.0 && tail.z > 1.5, "{tail:?}");
    let supports = catalogue.ground_supports();
    assert_eq!(supports.len(), 2);
    // The nine plan-box samples and nine contact-patch points a wheel.
    assert_eq!(supports[0].contacts().len(), 9 + 36);
    assert!(supports[1].contacts().iter().any(|c| (c.z - (1.95 + 0.13)).abs() < 1e-5));
}
