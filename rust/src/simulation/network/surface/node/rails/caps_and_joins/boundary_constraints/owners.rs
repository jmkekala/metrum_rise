// SPDX-License-Identifier: GPL-2.0-only

//! Owner lookup helpers for terminal-cap and side-join boundary constraints.

use super::*;

pub(super) fn terminal_cap_band_material_opposite_owner(
    mouth: &NodeInputMouth,
    cap_band: &NodeTerminalCapBand,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    if let Some(owner) =
        terminal_cap_generated_material_opposite_owner(cap_band, owner_by_kind_and_source)
    {
        return Some(owner);
    }
    match cap_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => adjacent_source_band_owner(
            mouth,
            cap_band.source_band_index,
            RoadSurfaceBandKind::Sidewalk,
            owner_by_kind_and_source,
        ),
        RoadSurfaceBandKind::Sidewalk => adjacent_source_band_owner(
            mouth,
            cap_band.source_band_index,
            RoadSurfaceBandKind::CurbOrShoulder,
            owner_by_kind_and_source,
        ),
        _ => None,
    }
}

fn terminal_cap_generated_material_opposite_owner(
    cap_band: &NodeTerminalCapBand,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    match cap_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => {
            cap_band
                .source_band_index
                .checked_add(1)
                .and_then(|source_band_index| {
                    owner_by_kind_and_source
                        .get(&(RoadSurfaceBandKind::Sidewalk, source_band_index))
                        .copied()
                })
        }
        RoadSurfaceBandKind::Sidewalk => {
            cap_band
                .source_band_index
                .checked_sub(1)
                .and_then(|source_band_index| {
                    owner_by_kind_and_source
                        .get(&(RoadSurfaceBandKind::CurbOrShoulder, source_band_index))
                        .copied()
                })
        }
        _ => None,
    }
}

pub(super) fn side_join_band_material_opposite_owner(
    mouth: &NodeInputMouth,
    side_join_band: &NodeInputSideJoinBand,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    match side_join_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => adjacent_source_band_owner(
            mouth,
            side_join_band.source_band_index,
            RoadSurfaceBandKind::Sidewalk,
            owner_by_kind_and_source,
        ),
        RoadSurfaceBandKind::Sidewalk => adjacent_source_band_owner(
            mouth,
            side_join_band.source_band_index,
            RoadSurfaceBandKind::CurbOrShoulder,
            owner_by_kind_and_source,
        ),
        _ => None,
    }
}

pub(super) fn inner_raised_step_opposite_owner_for_segment(
    mouth: &NodeInputMouth,
    source_band_index: usize,
    start: RoadVec2,
    end: RoadVec2,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    if let Some(owner) =
        endpoint_raised_step_opposite_owner(mouth, start, end, owner_by_kind_and_source)
    {
        return Some(owner);
    }
    adjacent_source_band_owner(
        mouth,
        source_band_index,
        RoadSurfaceBandKind::Carriageway,
        owner_by_kind_and_source,
    )
}

fn endpoint_raised_step_opposite_owner(
    mouth: &NodeInputMouth,
    start: RoadVec2,
    end: RoadVec2,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    let start_boundary = endpoint_boundary_index_for_point(mouth, start)?;
    let end_boundary = endpoint_boundary_index_for_point(mouth, end)?;
    let (lower_boundary, upper_boundary) = if start_boundary <= end_boundary {
        (start_boundary, end_boundary)
    } else {
        (end_boundary, start_boundary)
    };
    if lower_boundary + 1 != upper_boundary {
        return None;
    }
    let interval = mouth.band_intervals.get(lower_boundary)?;
    if !is_carriageway(interval.band_kind) {
        return None;
    }
    owner_by_kind_and_source
        .get(&(RoadSurfaceBandKind::Carriageway, interval.band_index))
        .copied()
}

fn adjacent_source_band_owner(
    mouth: &NodeInputMouth,
    source_band_index: usize,
    adjacent_kind: RoadSurfaceBandKind,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    let source = mouth.band_intervals.get(source_band_index)?;
    let mut adjacent_owner = None;
    for adjacent_index in [
        source_band_index.checked_sub(1),
        source_band_index.checked_add(1),
    ]
    .into_iter()
    .flatten()
    {
        let Some(adjacent) = mouth.band_intervals.get(adjacent_index) else {
            continue;
        };
        if adjacent.band_kind != adjacent_kind {
            continue;
        }
        let owner = owner_by_kind_and_source
            .get(&(adjacent.band_kind, adjacent.band_index))
            .copied()?;
        if adjacent_owner.replace(owner).is_some() {
            return None;
        }
    }
    (source.band_kind != adjacent_kind)
        .then_some(adjacent_owner)
        .flatten()
}

fn endpoint_boundary_index_for_point(mouth: &NodeInputMouth, point: RoadVec2) -> Option<usize> {
    let key = road_point_key(point);
    mouth
        .boundary_rails
        .iter()
        .find(|rail| road_point_key(xz(rail.endpoint_world)) == key)
        .map(|rail| rail.boundary_index)
}
