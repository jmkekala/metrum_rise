//! Canonical rail point-set lookup methods.

use super::*;

impl NodeRailCanonicalPointSet {
    pub(in crate::simulation::network::surface::node::ownership) fn canonicalized_point_for_owner(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> Result<NodeOwnershipPointKey, NodeBooleanOwnershipError> {
        match self.canonical_point_for_owner(owner, point) {
            Ok(Some(canonical)) => Ok(canonical),
            Ok(None) => Ok(point),
            Err(error) => Err(error),
        }
    }

    fn canonical_point_for_owner(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> Result<Option<NodeOwnershipPointKey>, NodeBooleanOwnershipError> {
        let Some(candidates) = self.canonical_candidates_for_owner(owner, point) else {
            return Ok(None);
        };
        if candidates.contains(&point) {
            return Ok(Some(point));
        }
        if candidates.len() == 1 {
            return Ok(candidates.iter().copied().next());
        }
        Err(
            NodeBooleanOwnershipError::AmbiguousCanonicalOwnedRegionVertex {
                owner,
                point_x_key: point.0,
                point_z_key: point.1,
                candidates: candidates.iter().copied().collect(),
            },
        )
    }

    fn canonical_candidates_for_owner(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> Option<&BTreeSet<NodeOwnershipPointKey>> {
        self.canonical_points_by_mm_key_by_owner
            .get(&owner)?
            .get(&ownership_mm_key(point))
    }
}
