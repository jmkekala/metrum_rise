//! Terminal-cap boundary constraint emission.

use super::owners::terminal_cap_band_material_opposite_owner;
use super::*;

mod curb;
mod paths;
mod sidewalk;

use curb::push_terminal_cap_curb_or_shoulder_boundary_constraints;
use sidewalk::push_terminal_cap_sidewalk_boundary_constraints;

pub(in crate::simulation::network::surface::node::rails::caps_and_joins) fn push_terminal_cap_band_boundary_constraints(
    mouth: &NodeInputMouth,
    cap_band: &NodeTerminalCapBand,
    owner: NodeBandOwner,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let opposite_owner =
        terminal_cap_band_material_opposite_owner(mouth, cap_band, owner_by_kind_and_source);
    match cap_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => {
            push_terminal_cap_curb_or_shoulder_boundary_constraints(
                mouth,
                cap_band,
                owner,
                opposite_owner,
                owner_by_kind_and_source,
                constraints,
            )
        }
        RoadSurfaceBandKind::Sidewalk => push_terminal_cap_sidewalk_boundary_constraints(
            mouth,
            cap_band,
            owner,
            opposite_owner,
            constraints,
        ),
        _ => Ok(()),
    }
}
