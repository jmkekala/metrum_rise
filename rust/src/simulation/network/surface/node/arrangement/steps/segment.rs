// SPDX-License-Identifier: GPL-2.0-only

//! Explicit vertical-step segment identity.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeExplicitVerticalStepSegment {
    start: NodeArrangementKey,
    end: NodeArrangementKey,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
}

pub(crate) fn explicit_vertical_step_segments_authorize_height_side_at_key(
    key: NodeArrangementKey,
    owner: NodeBandOwner,
    lower_side: bool,
    segments: &[NodeExplicitVerticalStepSegment],
) -> bool {
    segments
        .iter()
        .copied()
        .any(|segment| segment.authorizes_height_side_at_key(key, owner, lower_side))
}

impl NodeExplicitVerticalStepSegment {
    pub(crate) fn new(
        a: NodeArrangementKey,
        b: NodeArrangementKey,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
    ) -> Option<Self> {
        if a == b {
            return None;
        }
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        let (owner, opposite_owner) = if owner <= opposite_owner {
            (owner, opposite_owner)
        } else {
            (opposite_owner, owner)
        };
        Some(Self {
            start,
            end,
            owner,
            opposite_owner,
        })
    }

    pub(crate) fn start(self) -> NodeArrangementKey {
        self.start
    }

    pub(crate) fn end(self) -> NodeArrangementKey {
        self.end
    }

    pub(crate) fn owner(self) -> NodeBandOwner {
        self.owner
    }

    pub(crate) fn opposite_owner(self) -> NodeBandOwner {
        self.opposite_owner
    }

    pub(crate) fn authorizes_height_side_at_key(
        self,
        key: NodeArrangementKey,
        owner: NodeBandOwner,
        lower_side: bool,
    ) -> bool {
        key.lies_on_segment(self.start, self.end)
            && self.owner_matches_height_side(owner, lower_side)
    }

    pub(crate) fn owner_matches_height_side(self, owner: NodeBandOwner, lower_side: bool) -> bool {
        let Some(owner_rank) = raised_step_band_rank(self.owner.kind()) else {
            return false;
        };
        let Some(opposite_rank) = raised_step_band_rank(self.opposite_owner.kind()) else {
            return false;
        };
        match owner_rank.cmp(&opposite_rank) {
            std::cmp::Ordering::Less => {
                (lower_side && owner == self.owner) || (!lower_side && owner == self.opposite_owner)
            }
            std::cmp::Ordering::Greater => {
                (lower_side && owner == self.opposite_owner) || (!lower_side && owner == self.owner)
            }
            std::cmp::Ordering::Equal => false,
        }
    }
}
