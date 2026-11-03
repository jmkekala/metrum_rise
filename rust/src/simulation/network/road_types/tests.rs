// SPDX-License-Identifier: GPL-2.0-only

//! The layout check, widths, built-ins and name rules (`ROAD-101`).

use super::*;

const S: RoadPart = RoadPart::Sidewalk;
const F: RoadPart = RoadPart::CarLane { forward: true };
const B: RoadPart = RoadPart::CarLane { forward: false };

fn median(style: MedianStyle, width_m: f32) -> RoadPart {
    RoadPart::Median(RoadMedian { style, width_m })
}

fn grass(width_m: f32) -> RoadPart {
    median(MedianStyle::KerbedGrass, width_m)
}

fn refused(parts: &[RoadPart]) -> (&'static str, Option<usize>) {
    let refusal = check_layout(parts).expect_err("the layout is refused");
    (refusal.reason, refusal.part)
}

#[test]
fn every_layout_refusal_has_its_key_and_part() {
    let m = grass(3.0);
    let cases: &[(&[RoadPart], &str, Option<usize>)] = &[
        (&[RoadPart::Footpath, S], "layout_footpath_alone", Some(0)),
        (
            &[S, B, RoadPart::Footpath, F, S],
            "layout_footpath_alone",
            Some(2),
        ),
        (&[], "layout_no_car_lane", None),
        (&[S], "layout_sidewalks", None),
        // One side only: none or both.
        (&[B, F, S], "layout_sidewalks", None),
        (&[S, B, F], "layout_sidewalks", None),
        (&[S, B, S, F, S], "layout_sidewalks", Some(2)),
        (&[B, S, F], "layout_sidewalks", Some(1)),
        (&[B, B, m, m, F], "layout_two_medians", Some(3)),
        (&[F, B], "layout_lane_order", Some(1)),
        (&[m, B, F], "layout_median_at_edge", Some(0)),
        (&[S, B, m, m, F, S], "layout_two_medians", Some(3)),
        (&[S, F, B, S], "layout_lane_order", Some(2)),
        (&[S, B, F, B, S], "layout_lane_order", Some(3)),
        (&[S, S], "layout_no_car_lane", None),
        (&[S, m, S], "layout_no_car_lane", None),
        (&[S, F, F, F, F, F, S], "too_many_lanes", None),
        (&[S, B, B, B, B, B, F, S], "too_many_lanes", None),
        (
            &[S, B, grass(1.0), F, S],
            "median_width_out_of_range",
            Some(2),
        ),
        (
            &[S, B, grass(f32::NAN), F, S],
            "median_width_out_of_range",
            Some(2),
        ),
        (&[S, F, m, F, S], "median_requires_both_directions", Some(2)),
        (&[S, m, F, S], "median_requires_both_directions", Some(1)),
        (&[S, B, B, m, S], "median_requires_both_directions", Some(3)),
        (&[S, B, m, B, F, S], "layout_lanes_split", Some(2)),
        (&[S, B, F, m, F, S], "layout_lanes_split", Some(3)),
        (&[S, m, B, F, S], "layout_median_at_edge", Some(1)),
        (&[S, B, F, m, S], "layout_median_at_edge", Some(3)),
    ];
    for (parts, reason, part) in cases {
        assert_eq!(refused(parts), (*reason, *part), "{parts:?}");
    }
}

#[test]
fn valid_layouts_build_their_cross_section() {
    let paving = median(MedianStyle::KerbedPaving, 2.0);
    let RoadPart::Median(paving_median) = paving else {
        unreachable!()
    };
    let section = |fwd_lanes, bkw_lanes, median| RoadCrossSection {
        fwd_lanes,
        bkw_lanes,
        median,
        sidewalks: true,
    };
    let bare = |fwd_lanes, bkw_lanes, median| RoadCrossSection {
        sidewalks: false,
        ..section(fwd_lanes, bkw_lanes, median)
    };
    let cases: &[(&[RoadPart], RoadCrossSection)] = &[
        (&[RoadPart::Footpath], RoadCrossSection::WALKWAY),
        (&[B, F], bare(1, 1, None)),
        (&[F], bare(1, 0, None)),
        (&[B, B, paving, F, F], bare(2, 2, Some(paving_median))),
        (&[S, B, F, S], section(1, 1, None)),
        (&[S, F, S], section(1, 0, None)),
        (&[S, B, S], section(0, 1, None)),
        (&[S, B, F, F, S], section(2, 1, None)),
        (
            &[S, B, B, paving, F, F, S],
            section(2, 2, Some(paving_median)),
        ),
        (&[S, B, B, B, B, F, F, F, F, S], section(4, 4, None)),
    ];
    for (parts, expected) in cases {
        let section = check_layout(parts).unwrap();
        assert_eq!(section, *expected);
        // The canonical layout is the drawn one and checks back to the same cross-section.
        assert_eq!(section.parts(), *parts);
        assert_eq!(check_layout(&section.parts()), Ok(section));
    }
}

#[test]
fn widths_sum_to_the_edge_width_plus_sidewalks() {
    let (widths, carriageway, total) = layout_widths(&[S, B, B, grass(3.0), F, F, S]);
    assert_eq!(widths, [1.5, 3.5, 3.5, 3.0, 3.5, 3.5, 1.5]);
    assert_eq!((carriageway, total), (17.0, 20.0));
    let section = check_layout(&[S, B, B, grass(3.0), F, F, S]).unwrap();
    assert_eq!(
        carriageway,
        Edge::cross_section_width_m(section.fwd_lanes, section.bkw_lanes, section.median)
    );
    // A walkway is its footpath, as wide as a 0 + 0 edge.
    let (widths, carriageway, total) = layout_widths(&[RoadPart::Footpath]);
    assert_eq!((widths, carriageway, total), (vec![2.0], 2.0, 2.0));
    assert_eq!(carriageway, Edge::cross_section_width_m(0, 0, None));
    // Refused layouts still have widths for the builder to draw.
    assert_eq!(layout_widths(&[S, F, B, S]).2, 10.0);
    // Without sidewalks the roadbed builds a 0.75 m shoulder each side.
    let (widths, carriageway, total) = layout_widths(&[B, grass(3.0), F]);
    assert_eq!((widths, carriageway, total), (vec![3.5, 3.0, 3.5], 10.0, 11.5));
    let section = check_layout(&[B, grass(3.0), F]).unwrap();
    assert_eq!(
        carriageway,
        Edge::cross_section_width_m(section.fwd_lanes, section.bkw_lanes, section.median)
    );
}

#[test]
fn the_builtins_build_todays_roads_menu() {
    let expected = [
        ("Walkway", 0, 0),
        ("2-Lane Road", 1, 1),
        ("4-Lane Road", 2, 2),
        ("One-Way 1L", 1, 0),
        ("One-Way 2L", 2, 0),
    ];
    assert_eq!(BUILTIN_ROAD_TYPES.len(), expected.len());
    for (builtin, (name, fwd_lanes, bkw_lanes)) in BUILTIN_ROAD_TYPES.iter().zip(expected) {
        let ty = RoadType::from_builtin(builtin);
        assert_eq!(ty.name, name);
        assert!(ty.builtin && ty.id.starts_with("builtin:"));
        let section = check_layout(&ty.parts).unwrap();
        assert_eq!(
            section,
            RoadCrossSection {
                fwd_lanes,
                bkw_lanes,
                median: None,
                sidewalks: fwd_lanes > 0,
            }
        );
        assert_eq!(ty.usable(), Some(section));
        // Every built-in draws today's edge: 50 km/h (none for the walkway), sidewalks, zoning.
        assert_eq!(ty.speed_kmh, if fwd_lanes > 0 { 50 } else { 0 });
        assert!(ty.zoning);
        if fwd_lanes > 0 {
            let attributes = RoadAttributes::new(ty.speed_kmh, section.sidewalks, ty.zoning);
            assert_eq!(attributes, RoadAttributes::DEFAULT);
        }
    }
    assert_eq!(
        RoadAttributes::DEFAULT.speed_ms.to_bits(),
        RoadAttributes::new(50, true, true).speed_ms.to_bits()
    );
    assert_eq!(RoadAttributes::speed_kmh_of(config::DEFAULT_URBAN_ROAD_SPEED_MS), 50);
    let types: Vec<RoadType> = BUILTIN_ROAD_TYPES
        .iter()
        .map(RoadType::from_builtin)
        .collect();
    let id_of = |section, speed| type_of(&types, section, speed).map(|ty| ty.id.as_str());
    assert_eq!(id_of(RoadCrossSection::WALKWAY, 50), Some("builtin:walkway"));
    let two_lane = check_layout(&[S, B, F, S]).unwrap();
    assert_eq!(id_of(two_lane, 50), Some("builtin:road_2l"));
    // Sidewalks and speed are part of the match.
    assert_eq!(id_of(two_lane, 60), None);
    assert_eq!(id_of(check_layout(&[B, F]).unwrap(), 50), None);
    let divided = check_layout(&[S, B, grass(3.0), F, S]).unwrap();
    assert!(type_of(&types, divided, 50).is_none());
}

#[test]
fn speeds_are_checked_on_the_10_kmh_grid() {
    let road = check_layout(&[S, B, F, S]).unwrap();
    for speed in [20, 50, 80, 120] {
        assert_eq!(check_speed_kmh(road, speed), Ok(speed as u16));
    }
    for speed in [0, 10, 15, 125, 130, -20, 65_556] {
        assert_eq!(check_speed_kmh(road, speed), Err("speed_out_of_range"));
    }
    assert_eq!(check_speed_kmh(RoadCrossSection::WALKWAY, 999), Ok(0));
    let tool = RoadAttributes::from_tool(100, false, false).unwrap();
    assert_eq!(
        tool.speed_ms.to_bits(),
        (100.0_f32 * config::KMH_TO_MPS).to_bits()
    );
    assert_eq!(RoadAttributes::speed_kmh_of(tool.speed_ms), 100);
    assert_eq!(
        RoadAttributes::from_tool(5, true, true),
        Err("speed_out_of_range")
    );
}

#[test]
fn names_are_trimmed_bounded_and_compared_ignoring_case() {
    assert_eq!(clean_name("  Boulevard \t"), Ok("Boulevard"));
    assert_eq!(clean_name("   "), Err("name_empty"));
    assert_eq!(clean_name(&"é".repeat(40)), Ok("é".repeat(40).as_str()));
    assert_eq!(clean_name(&"é".repeat(41)), Err("name_too_long"));
    assert_eq!(clean_name("Main\nStreet"), Err("name_invalid_characters"));
    assert_eq!(clean_name("Main\u{7}"), Err("name_invalid_characters"));
    assert!(same_name("Kävelykatu", " KÄVELYKATU "));
    assert!(same_name("2-lane road", "2-Lane Road"));
    assert!(!same_name("Road", "Roads"));
}

#[test]
fn suggested_names_describe_the_layout_and_skip_taken_ones() {
    let section = |parts: &[RoadPart]| check_layout(parts).unwrap();
    let divided = section(&[S, B, B, grass(3.0), F, F, S]);
    assert_eq!(suggested_name(divided, |_| false), "2+2 Grass 3 m");
    let painted = section(&[S, B, median(MedianStyle::Painted, 1.5), F, S]);
    assert_eq!(suggested_name(painted, |_| false), "1+1 Painted 1.5 m");
    assert_eq!(suggested_name(section(&[S, F, F, S]), |_| false), "2+0");
    let taken = ["2+2 grass 3 m", "2+2 Grass 3 m (2)"];
    let is_taken = |name: &str| taken.iter().any(|taken| same_name(taken, name));
    assert_eq!(suggested_name(divided, is_taken), "2+2 Grass 3 m (3)");
    assert_eq!(
        suggested_name(RoadCrossSection::WALKWAY, |name| same_name(name, "Walkway")),
        "Walkway (2)"
    );
}
