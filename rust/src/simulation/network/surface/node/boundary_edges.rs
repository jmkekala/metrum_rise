//! Canonical boundary edge keys for node export.

use super::*;

pub(super) fn normalized_arrangement_boundary_segment_key(
    start: ArrangementBoundaryPointKey,
    end: ArrangementBoundaryPointKey,
) -> (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey) {
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}
