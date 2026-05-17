//! Canonical boundary edge keys for node export.

use super::*;

pub(super) fn normalized_arrangement_boundary_segment_key(
    start: Vector3,
    end: Vector3,
) -> (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey) {
    let start = ArrangementBoundaryPointKey::from_world(start);
    let end = ArrangementBoundaryPointKey::from_world(end);
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}
