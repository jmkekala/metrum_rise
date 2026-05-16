//! Canonical semantic ordering for road-surface band kinds.

use super::RoadSurfaceBandKind;

pub(crate) fn band_kind_sort_key(kind: RoadSurfaceBandKind) -> u8 {
    match kind {
        RoadSurfaceBandKind::Carriageway => 0,
        RoadSurfaceBandKind::CurbOrShoulder => 1,
        RoadSurfaceBandKind::Sidewalk => 2,
        RoadSurfaceBandKind::Footpath => 3,
        RoadSurfaceBandKind::Median => 4,
        RoadSurfaceBandKind::Parking => 5,
        RoadSurfaceBandKind::CycleTrack => 6,
        RoadSurfaceBandKind::TramReservation => 7,
    }
}

pub(crate) fn raised_step_band_rank(kind: RoadSurfaceBandKind) -> Option<u8> {
    match kind {
        RoadSurfaceBandKind::Carriageway => Some(0),
        RoadSurfaceBandKind::CurbOrShoulder => Some(1),
        RoadSurfaceBandKind::Sidewalk => Some(2),
        RoadSurfaceBandKind::Footpath
        | RoadSurfaceBandKind::Median
        | RoadSurfaceBandKind::Parking
        | RoadSurfaceBandKind::CycleTrack
        | RoadSurfaceBandKind::TramReservation => None,
    }
}

pub(crate) fn raised_step_kinds_can_contact(
    a: RoadSurfaceBandKind,
    b: RoadSurfaceBandKind,
) -> bool {
    let Some(a_rank) = raised_step_band_rank(a) else {
        return false;
    };
    let Some(b_rank) = raised_step_band_rank(b) else {
        return false;
    };
    a_rank.abs_diff(b_rank) == 1
}

pub(crate) fn ordered_raised_step_kinds(
    a: RoadSurfaceBandKind,
    b: RoadSurfaceBandKind,
) -> Option<(RoadSurfaceBandKind, RoadSurfaceBandKind)> {
    let a_rank = raised_step_band_rank(a)?;
    let b_rank = raised_step_band_rank(b)?;
    if a_rank == b_rank {
        return None;
    }
    if a_rank < b_rank {
        Some((a, b))
    } else {
        Some((b, a))
    }
}

pub(crate) fn raised_step_requires_exact_constraint_span(
    a: RoadSurfaceBandKind,
    b: RoadSurfaceBandKind,
) -> bool {
    matches!(
        ordered_raised_step_kinds(a, b),
        Some((
            RoadSurfaceBandKind::Carriageway,
            RoadSurfaceBandKind::CurbOrShoulder
        ))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raised_step_ranks_are_canonical_and_exclude_footpath() {
        assert_eq!(
            raised_step_band_rank(RoadSurfaceBandKind::Carriageway),
            Some(0)
        );
        assert_eq!(
            raised_step_band_rank(RoadSurfaceBandKind::CurbOrShoulder),
            Some(1)
        );
        assert_eq!(
            raised_step_band_rank(RoadSurfaceBandKind::Sidewalk),
            Some(2)
        );
        assert_eq!(raised_step_band_rank(RoadSurfaceBandKind::Footpath), None);
    }

    #[test]
    fn raised_step_contact_requires_adjacent_canonical_ranks() {
        assert!(raised_step_kinds_can_contact(
            RoadSurfaceBandKind::Carriageway,
            RoadSurfaceBandKind::CurbOrShoulder
        ));
        assert!(raised_step_kinds_can_contact(
            RoadSurfaceBandKind::CurbOrShoulder,
            RoadSurfaceBandKind::Sidewalk
        ));
        assert!(!raised_step_kinds_can_contact(
            RoadSurfaceBandKind::Carriageway,
            RoadSurfaceBandKind::Sidewalk
        ));
        assert!(!raised_step_kinds_can_contact(
            RoadSurfaceBandKind::Footpath,
            RoadSurfaceBandKind::CurbOrShoulder
        ));
    }

    #[test]
    fn exact_constraint_span_is_only_for_carriageway_curb_contacts() {
        assert!(raised_step_requires_exact_constraint_span(
            RoadSurfaceBandKind::Carriageway,
            RoadSurfaceBandKind::CurbOrShoulder
        ));
        assert!(!raised_step_requires_exact_constraint_span(
            RoadSurfaceBandKind::CurbOrShoulder,
            RoadSurfaceBandKind::Sidewalk
        ));
        assert!(!raised_step_requires_exact_constraint_span(
            RoadSurfaceBandKind::Footpath,
            RoadSurfaceBandKind::CurbOrShoulder
        ));
    }
}
