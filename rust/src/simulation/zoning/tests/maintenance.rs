//! Zoning maintenance behavior tests.

use super::helpers::{make_straight_road, make_zoning};
use crate::simulation::zoning::ZoneType;

#[test]
fn test_parcel_edge_compaction_remaps_and_drops_missing_edges() {
    let (graph, edge_idx) = make_straight_road();
    let mut z = make_zoning();
    let residential = z
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();

    z.place_or_rezone_default_parcel_at(0.0, -20.0, residential, &graph)
        .expect("parcel");

    let mut map = std::collections::HashMap::new();
    map.insert(edge_idx, 7);
    z.update_edge_indices(&map);
    assert_eq!(z.parcels()[0].edge_idx(), 7);

    z.update_edge_indices(&std::collections::HashMap::new());
    assert!(z.parcels().is_empty());
}
