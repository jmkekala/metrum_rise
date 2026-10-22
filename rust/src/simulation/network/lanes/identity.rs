// SPDX-License-Identifier: GPL-2.0-only

//! Lane identities that survive a full [`LaneSystem::rebuild`], which renumbers every lane.

use super::LaneSystem;
use std::collections::{HashMap, HashSet};

/// A road lane by its owning edge, direction and lane index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RoadLaneKey {
    edge_id: usize,
    is_fwd: bool,
    lane_idx: i8,
}

/// A lane independent of its index in [`LaneSystem::lanes`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum LaneIdentity {
    /// A physical lane along a road edge.
    Road(RoadLaneKey),
    /// A junction connector, which joins exactly one incoming and one outgoing road lane.
    Connector {
        node_id: usize,
        from: RoadLaneKey,
        to: RoadLaneKey,
    },
}

impl LaneSystem {
    fn road_lane_key(&self, lane_id: usize) -> Option<RoadLaneKey> {
        let lane = self.lanes.get(lane_id)?;
        // A tombstoned or replaced lane is no longer in its edge's live list.
        self.edge_lanes
            .get(&lane.edge_id)?
            .contains(&lane_id)
            .then_some(RoadLaneKey {
                edge_id: lane.edge_id,
                is_fwd: lane.is_fwd,
                lane_idx: lane.lane_idx,
            })
    }

    /// Returns the identity of each live lane in `lane_ids`; dead or unknown ids are omitted.
    ///
    /// Connectors store no predecessor, so one pass over the live road lanes finds them:
    /// O(L) when any connector is requested, otherwise O(|lane_ids|).
    pub(crate) fn lane_identities(
        &self,
        lane_ids: &HashSet<usize>,
    ) -> HashMap<usize, LaneIdentity> {
        let mut identities = HashMap::with_capacity(lane_ids.len());
        let mut connectors = false;
        for &lane_id in lane_ids {
            match self.lanes.get(lane_id) {
                Some(lane) if lane.edge_id != usize::MAX => {
                    if let Some(key) = self.road_lane_key(lane_id) {
                        identities.insert(lane_id, LaneIdentity::Road(key));
                    }
                }
                Some(_) => connectors = true,
                None => {}
            }
        }
        if !connectors {
            return identities;
        }
        for ids in self.edge_lanes.values() {
            for &from_id in ids {
                for &conn_id in self.lanes[from_id].next_lanes.iter() {
                    if !lane_ids.contains(&conn_id) {
                        continue;
                    }
                    let conn = &self.lanes[conn_id];
                    let to = conn
                        .next_lanes
                        .first()
                        .and_then(|&to| self.road_lane_key(to));
                    if let (Some(from), Some(to)) = (self.road_lane_key(from_id), to) {
                        identities.insert(
                            conn_id,
                            LaneIdentity::Connector {
                                node_id: conn.node_id,
                                from,
                                to,
                            },
                        );
                    }
                }
            }
        }
        identities
    }

    fn resolve_road_lane(&self, key: RoadLaneKey) -> Option<usize> {
        self.edge_lanes
            .get(&key.edge_id)?
            .iter()
            .copied()
            .find(|&id| {
                let lane = &self.lanes[id];
                lane.is_fwd == key.is_fwd && lane.lane_idx == key.lane_idx
            })
    }

    /// Returns the live lane with `identity`, or `None` when the rebuild removed it.
    pub(crate) fn resolve_lane_identity(&self, identity: LaneIdentity) -> Option<usize> {
        match identity {
            LaneIdentity::Road(key) => self.resolve_road_lane(key),
            LaneIdentity::Connector { node_id, from, to } => {
                let from = self.resolve_road_lane(from)?;
                let to = self.resolve_road_lane(to)?;
                self.lanes[from].next_lanes.iter().copied().find(|&id| {
                    let lane = &self.lanes[id];
                    lane.edge_id == usize::MAX
                        && lane.node_id == node_id
                        && lane.next_lanes.first() == Some(&to)
                })
            }
        }
    }
}
