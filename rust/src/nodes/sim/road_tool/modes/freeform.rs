// SPDX-License-Identifier: GPL-3.0-or-later

//! Freeform (`ROAD-112` round N, plan 2.2): a draft of cubic Bezier spans through the nodes the
//! player places, with handles the player can edit before the road is built. No freehand: the
//! nodes are clicks.
//!
//! **Placing.** Each click places a node (the session keeps the positions in its placed points);
//! the live cursor is the draft's provisional last node. A click on the last node again,
//! `confirm`, a node snapped to the network or the map edge, or the sixteenth node ends the
//! placing and starts the **edit phase**: a press within [`GRIP_REACH_M`] of a node or a handle
//! grabs it and the pointer drags it until the release; any other press, or `confirm`, sends
//! the stroke; `cancel` drops the draft and leaves nothing.
//!
//! **Handles.** Every node has an inward and an outward handle, kept as offsets from the node,
//! so a dragged node carries its handles. Until the player drags one of its handles a node's
//! handles are automatic, recomputed from its neighbours on every pose: an inner node's are
//! `+-(P[i+1] - P[i-1]) / 6` (the Catmull-Rom tangent, so the spans join tangent-continuously),
//! an end's runs half way towards its neighbour's handle point (a two-node draft is the straight
//! line with thirds). Dragging a handle makes the node's handles the player's; the opposite
//! handle turns with it, keeping its own length, so the joint stays collinear, unless the drag
//! breaks it (Ctrl): a broken node's handles move apart from then on. A chained stroke's start is
//! where the last road ended, so it is not grabbed. With Continuous on (round CT) a chained
//! start's handle is locked to the carried direction (the last road's end tangent): automatic or
//! dragged, only its length changes, and Ctrl does not break it (no other chained start can
//! leave its carried direction either).
//!
//! Everything is fixed-size: the spans are rebuilt in the session's reused path, so a pose
//! allocates nothing. `O(nodes)` per pose, plus the kernel's Bezier work per span.

use super::super::frame::{GizmoHandleKind, GizmoLineClass, Gizmos};
use crate::simulation::network::guide::{CubicBezier, DVec2, GuideError, GuidePath, GuideSegment};
use crate::simulation::network::interaction::NetworkSnapTarget;
use godot::prelude::Vector3;

/// Most nodes a freeform draft takes (15 spans); the sixteenth click ends the placing.
pub const MAX_NODES: usize = 16;
/// How near a press must be to a node or a handle to grab it, in world metres (fixed, as every
/// tool tolerance: no zoom dependence).
pub const GRIP_REACH_M: f64 = 4.0;

/// What a press in the edit phase grabbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    /// A node (it moves with its handles).
    Node(usize),
    /// A node's inward handle (towards the previous node).
    Inward(usize),
    /// A node's outward handle (towards the next node).
    Outward(usize),
}

// One node's handles as offsets from the node.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Handles {
    inward: DVec2,
    outward: DVec2,
}

/// The freeform draft's state beside the session's placed points: handles, which nodes the
/// player edited or broke, where each node met the network, the phase and the grip.
#[derive(Clone, Debug)]
pub(crate) struct Freeform {
    // The last pose's handles, one entry per node of the draft (the live node included).
    handles: [Handles; MAX_NODES + 1],
    // The node's handles are the player's (not recomputed).
    manual: [bool; MAX_NODES],
    // The node's handles move apart (a corner).
    broken: [bool; MAX_NODES],
    // The network target and map-edge snap each placed node took (the chain rules read the
    // last node's, the HUD its road).
    targets: [Option<NetworkSnapTarget>; MAX_NODES],
    border: [bool; MAX_NODES],
    // The unit direction the start's outward handle is locked to (a continuous chain's carried
    // end tangent).
    start_direction: Option<DVec2>,
    editing: bool,
    grip: Option<Grip>,
}

impl Default for Freeform {
    fn default() -> Self {
        Self {
            handles: [Handles::default(); MAX_NODES + 1],
            manual: [false; MAX_NODES],
            broken: [false; MAX_NODES],
            targets: [None; MAX_NODES],
            border: [false; MAX_NODES],
            start_direction: None,
            editing: false,
            grip: None,
        }
    }
}

impl Freeform {
    /// Drops the draft's state (the session drops its placed points).
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Records node `index` as placed (or dragged) with the snap it took.
    pub(crate) fn place(&mut self, index: usize, target: Option<NetworkSnapTarget>, border: bool) {
        self.targets[index] = target;
        self.border[index] = border;
        if !self.editing {
            self.manual[index] = false;
            self.broken[index] = false;
        }
    }

    /// Locks the start's outward handle to the unit `direction` (`None` frees it): a
    /// continuous chain's carried end tangent.
    pub(crate) fn lock_start(&mut self, direction: Option<DVec2>) {
        self.start_direction = direction;
    }

    /// Whether the draft is in its edit phase.
    pub(crate) fn editing(&self) -> bool {
        self.editing
    }

    /// Starts the edit phase.
    pub(crate) fn start_editing(&mut self) {
        self.editing = true;
    }

    /// The grabbed grip, if a drag is on.
    pub(crate) fn grip(&self) -> Option<Grip> {
        self.grip
    }

    /// Where node `index` met the network and whether it took the map edge.
    pub(crate) fn snap_of(&self, index: usize) -> (Option<NetworkSnapTarget>, bool) {
        (self.targets[index], self.border[index])
    }

    /// The grip nearest `at` within [`GRIP_REACH_M`] among `nodes` (the placed nodes) and their
    /// handles as the last pose built them; nodes before handles and lower indices first on a
    /// tie. A `chained` start is not offered. `O(nodes)`.
    pub(crate) fn grip_at(&self, nodes: &[DVec2], at: DVec2, chained: bool) -> Option<Grip> {
        let last = nodes.len().checked_sub(1)?;
        let mut best = None;
        let mut best_sq = GRIP_REACH_M * GRIP_REACH_M;
        let mut consider = |grip: Grip, point: DVec2| {
            let distance_sq = point.distance_squared(at);
            if distance_sq <= best_sq && best.is_none_or(|_| distance_sq < best_sq) {
                best = Some(grip);
                best_sq = distance_sq;
            }
        };
        for (index, node) in nodes.iter().enumerate() {
            if !(chained && index == 0) {
                consider(Grip::Node(index), *node);
            }
        }
        for (index, node) in nodes.iter().enumerate() {
            let handles = self.handles[index];
            if index > 0 {
                consider(Grip::Inward(index), *node + handles.inward);
            }
            if index < last {
                consider(Grip::Outward(index), *node + handles.outward);
            }
        }
        best
    }

    /// Grabs `grip`: a handle's node takes the handles the last pose built as the player's.
    pub(crate) fn grab(&mut self, grip: Grip) {
        if let Grip::Inward(index) | Grip::Outward(index) = grip {
            self.manual[index] = true;
        }
        self.grip = Some(grip);
    }

    /// Ends a drag.
    pub(crate) fn release(&mut self) {
        self.grip = None;
    }

    /// Moves the grabbed handle to `at` (a node's grip is the session's to move, with its
    /// snap). The opposite handle turns to stay collinear, keeping its length, unless the node
    /// is broken or `break_joint` (Ctrl) breaks it now. A handle dragged onto its node keeps its
    /// last place (a span needs a non-zero end handle). A locked start's handle takes the drag's
    /// length along its direction (not behind the node: it keeps its last place).
    pub(crate) fn drag_handle(&mut self, nodes: &[DVec2], at: DVec2, break_joint: bool) {
        let (index, outward) = match self.grip {
            Some(Grip::Inward(index)) => (index, false),
            Some(Grip::Outward(index)) => (index, true),
            _ => return,
        };
        let offset = at - nodes[index];
        if offset == DVec2::ZERO {
            return;
        }
        if let Some(direction) = self.start_direction.filter(|_| index == 0) {
            let length = offset.dot(direction);
            if length > 0.0 {
                self.handles[0].outward = direction * length;
            }
            return;
        }
        if break_joint {
            self.broken[index] = true;
        }
        let handles = &mut self.handles[index];
        let (moved, opposite) = if outward {
            (&mut handles.outward, &mut handles.inward)
        } else {
            (&mut handles.inward, &mut handles.outward)
        };
        *moved = offset;
        if !self.broken[index] {
            let length = opposite.length();
            let length = if length > 0.0 {
                length
            } else {
                offset.length()
            };
            *opposite = -offset.normalize() * length;
        }
    }

    /// Fills `path` with the spans through `nodes` (the placed nodes, then the live one while
    /// placing): automatic handles recomputed, the player's kept. `Degenerate` when two
    /// consecutive nodes coincide or a handle has no length. `O(nodes)`, no allocation once
    /// `path` holds the spans.
    pub(crate) fn shape(
        &mut self,
        nodes: &[DVec2],
        path: &mut GuidePath,
    ) -> Result<(), GuideError> {
        path.clear();
        let count = nodes.len();
        if count < 2 {
            return Err(GuideError::Degenerate);
        }
        let manual = |index: usize| index < MAX_NODES && self.manual[index];
        // Inner nodes first: the ends aim at their neighbour's handle.
        for index in 1..count - 1 {
            if !manual(index) {
                let tangent = (nodes[index + 1] - nodes[index - 1]) / 6.0;
                self.handles[index] = Handles {
                    inward: -tangent,
                    outward: tangent,
                };
            }
        }
        // An end aims at its neighbour's handle; two automatic ends make the line in thirds. A
        // locked start keeps the automatic length along its direction, and the end aims at it.
        let last = count - 1;
        if !manual(0) {
            let outward = if count > 2 || manual(last) {
                (nodes[1] + self.handles[1].inward - nodes[0]) / 2.0
            } else {
                (nodes[1] - nodes[0]) / 3.0
            };
            self.handles[0].outward = match self.start_direction {
                Some(direction) => direction * outward.length(),
                None => outward,
            };
        }
        let start_locked = self.start_direction.is_some();
        if !manual(last) {
            self.handles[last].inward = if count > 2 || manual(0) || start_locked {
                (nodes[last - 1] + self.handles[last - 1].outward - nodes[last]) / 2.0
            } else {
                (nodes[0] - nodes[1]) / 3.0
            };
        }
        for span in 0..last {
            let (from, to) = (nodes[span], nodes[span + 1]);
            path.push(GuideSegment::Bezier(CubicBezier::new(
                from,
                from + self.handles[span].outward,
                to + self.handles[span + 1].inward,
                to,
            )?));
        }
        Ok(())
    }

    /// The draft's gizmos: the start (a chain's start when `chained`), the inner nodes as joints,
    /// the end (the live end while placing, as the cursor), and in the edit phase each node's
    /// handles with their stems; the grabbed grip shows as the cursor. `nodes` are the draft's
    /// nodes with their heights (the live one last while placing).
    pub(crate) fn push_gizmos(&self, gizmos: &mut Gizmos, nodes: &[Vector3], chained: bool) {
        let last = nodes.len().saturating_sub(1);
        let grabbed = |grip: Grip| self.editing && self.grip == Some(grip);
        for (index, node) in nodes.iter().enumerate() {
            let kind = match index {
                _ if grabbed(Grip::Node(index)) => GizmoHandleKind::Cursor,
                0 if chained => GizmoHandleKind::ChainStart,
                0 => GizmoHandleKind::Endpoint,
                _ if index == last && !self.editing => GizmoHandleKind::Cursor,
                _ if index == last => GizmoHandleKind::Endpoint,
                _ => GizmoHandleKind::Joint,
            };
            gizmos.push_handle(kind, *node);
        }
        if !self.editing {
            return;
        }
        let at = |node: Vector3, offset: DVec2| {
            Vector3::new(node.x + offset.x as f32, node.y, node.z + offset.y as f32)
        };
        for (index, node) in nodes.iter().enumerate() {
            let handles = self.handles[index];
            let inward = (index > 0).then(|| at(*node, handles.inward));
            let outward = (index < last).then(|| at(*node, handles.outward));
            gizmos.push_line(
                GizmoLineClass::Tangent,
                inward.into_iter().chain([*node]).chain(outward),
            );
            for (point, grip) in [
                (inward, Grip::Inward(index)),
                (outward, Grip::Outward(index)),
            ] {
                if let Some(point) = point {
                    let kind = if grabbed(grip) {
                        GizmoHandleKind::Cursor
                    } else {
                        GizmoHandleKind::Handle
                    };
                    gizmos.push_handle(kind, point);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(x: f64, y: f64) -> DVec2 {
        DVec2::new(x, y)
    }

    // A two-node draft is the straight line; three collinear even nodes stay on the line.
    #[test]
    fn automatic_handles_keep_a_straight_draft_straight() {
        let mut draft = Freeform::default();
        let mut path = GuidePath::new();
        draft
            .shape(&[d(0.0, 0.0), d(30.0, 0.0)], &mut path)
            .unwrap();
        assert!(path.min_radius().is_infinite());
        draft
            .shape(&[d(0.0, 0.0), d(30.0, 0.0), d(60.0, 0.0)], &mut path)
            .unwrap();
        assert_eq!(path.segments().len(), 2);
        assert!(path.min_radius().is_infinite());
        assert_eq!(path.start(), Some(d(0.0, 0.0)));
        assert_eq!(path.end(), Some(d(60.0, 0.0)));
    }

    // Automatic inner handles join the spans tangent-continuously.
    #[test]
    fn automatic_joints_are_tangent_continuous() {
        let mut draft = Freeform::default();
        let mut path = GuidePath::new();
        let nodes = [d(0.0, 0.0), d(40.0, 25.0), d(90.0, -10.0), d(130.0, 30.0)];
        draft.shape(&nodes, &mut path).unwrap();
        for pair in path.segments().windows(2) {
            let (a, b) = (pair[0].end_tangent(), pair[1].start_tangent());
            assert!(a.perp_dot(b).abs() < 1e-12 && a.dot(b) > 0.0);
        }
    }

    // Coincident nodes have no span.
    #[test]
    fn coincident_nodes_are_degenerate() {
        let mut draft = Freeform::default();
        let mut path = GuidePath::new();
        assert_eq!(
            draft.shape(&[d(5.0, 5.0), d(5.0, 5.0)], &mut path),
            Err(GuideError::Degenerate)
        );
    }
}
