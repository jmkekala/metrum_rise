// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic owner assignment helpers for generated node rails.

use super::super::arrangement::NodeBandOwner;
use super::super::input::NodeArrangementInput;
use super::super::joins::NodeInputSideJoinBand;
use super::super::terminal::NodeTerminalCapBand;
use super::{NodeGeneratedContour, NodeGeneratedContourKind, RoadSurfaceBandKind};
use std::collections::BTreeMap;

pub(super) struct MouthOwners {
    pub(super) band_owners: Vec<NodeBandOwner>,
    pub(super) terminal_cap_band_owners: Vec<NodeBandOwner>,
    pub(super) side_join_band_owners: Vec<NodeBandOwner>,
}

pub(super) fn generated_contour_band_kind(
    contour: &NodeGeneratedContour,
) -> Option<RoadSurfaceBandKind> {
    match contour.kind {
        NodeGeneratedContourKind::Band { kind } => Some(kind),
        NodeGeneratedContourKind::FullRoadbed => None,
    }
}
pub(super) fn owners_by_mouth(
    input: &NodeArrangementInput,
    terminal_cap_bands_by_mouth: &[Vec<NodeTerminalCapBand>],
    side_join_bands_by_mouth: &[Vec<NodeInputSideJoinBand>],
) -> Vec<MouthOwners> {
    let mut next_owner_index = 0usize;
    input
        .mouths
        .iter()
        .enumerate()
        .map(|(mouth_index, mouth)| {
            let band_owners: Vec<NodeBandOwner> = mouth
                .band_intervals
                .iter()
                .map(|interval| {
                    let owner = NodeBandOwner::new(interval.band_kind, next_owner_index);
                    next_owner_index += 1;
                    owner
                })
                .collect();
            let mut terminal_owner_by_source =
                BTreeMap::<(RoadSurfaceBandKind, usize), NodeBandOwner>::new();
            for (interval, owner) in mouth.band_intervals.iter().zip(&band_owners) {
                terminal_owner_by_source.insert((interval.band_kind, interval.band_index), *owner);
            }
            let side_join_bands = side_join_bands_by_mouth
                .get(mouth_index)
                .map_or(&[] as &[NodeInputSideJoinBand], Vec::as_slice);
            let terminal_cap_bands = terminal_cap_bands_by_mouth
                .get(mouth_index)
                .map_or(&[] as &[NodeTerminalCapBand], Vec::as_slice);
            let terminal_cap_band_owners = terminal_cap_bands
                .iter()
                .map(|cap_band| {
                    let key = (cap_band.band_kind, cap_band.source_band_index);
                    if let Some(owner) = terminal_owner_by_source.get(&key).copied() {
                        owner
                    } else {
                        let owner = NodeBandOwner::new(cap_band.band_kind, next_owner_index);
                        next_owner_index += 1;
                        terminal_owner_by_source.insert(key, owner);
                        owner
                    }
                })
                .collect();
            let side_join_band_owners = side_join_bands
                .iter()
                .map(|side_join_band| {
                    let key = (side_join_band.band_kind, side_join_band.source_band_index);
                    if let Some(owner) = terminal_owner_by_source.get(&key).copied() {
                        owner
                    } else {
                        let owner = NodeBandOwner::new(side_join_band.band_kind, next_owner_index);
                        next_owner_index += 1;
                        terminal_owner_by_source.insert(key, owner);
                        owner
                    }
                })
                .collect();
            MouthOwners {
                band_owners,
                terminal_cap_band_owners,
                side_join_band_owners,
            }
        })
        .collect()
}
pub(super) fn boundary_owners(
    boundary_index: usize,
    band_owners: &[NodeBandOwner],
) -> (Option<NodeBandOwner>, Option<NodeBandOwner>) {
    let left_owner = boundary_index
        .checked_sub(1)
        .and_then(|index| band_owners.get(index))
        .copied();
    let right_owner = band_owners.get(boundary_index).copied();
    match (left_owner, right_owner) {
        (Some(left_owner), Some(right_owner)) => (Some(left_owner), Some(right_owner)),
        (Some(owner), None) | (None, Some(owner)) => (Some(owner), None),
        (None, None) => (None, None),
    }
}
pub(super) fn is_carriageway(kind: RoadSurfaceBandKind) -> bool {
    kind == RoadSurfaceBandKind::Carriageway
}
