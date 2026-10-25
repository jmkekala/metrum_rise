// SPDX-License-Identifier: GPL-2.0-only

//! The road tool's versioned context (`ROAD-68-E`): the immutable view the simulation publishes
//! after each edit without copying the city, and the preview's private graph replica, kept equal
//! to the live graph by copying only the records whose pieces the roadbed rebuilt.
//!
//! The view shares the terrain and water copies until their own generations change, shares the
//! roadbed in `O(1)`, and takes its graph from the routing rebuild's immutable copy after a
//! commit (the one graph copy a commit makes, for the CCH job). The replica follows by
//! [`RegionGraph::restore_undo_delta_local`] of the rebuilt owners' records, in publication
//! order. Docs: `docs/roads.md` "Preview, commit, undo and load".

use std::sync::{Arc, Mutex, RwLock};

use super::road_preview::{RoadPreview, RoadToolQuerySnapshot, road_tool_snapshot_from_core};
use super::road_terrain_inputs::RoadTerrainInputs;
use super::state::SimCore;
use crate::simulation::network::graph::{RegionGraph, RegionGraphUndoDelta};

/// What the simulation last published to the road tool, and when.
#[derive(Default)]
pub(crate) struct RoadToolPublisher {
    sequence: u64,
    published: Option<PublishedView>,
    // An immutable copy of the live graph an editor already made for its routing rebuild,
    // offered to the next publication of the same command.
    offered_graph: Option<Arc<RegionGraph>>,
}

impl RoadToolPublisher {
    /// Offers the routing rebuild's graph copy to the publication that ends the same command
    /// (undo, bulldoze), so the view shares it instead of copying the graph again. The command
    /// must not change the graph after the copy; debug builds check the copy at publication.
    pub(crate) fn offer_graph(&mut self, graph: Arc<RegionGraph>) {
        self.offered_graph = Some(graph);
    }
}

struct PublishedView {
    view: RoadToolQuerySnapshot,
    terrain_generations: (u64, u64),
    water_generation: u64,
    graph_counts: (usize, usize),
    role_revision: u64,
}

/// How the preview's replica follows one publication.
pub(crate) enum ReplicaSync {
    /// The graph did not change.
    Unchanged,
    /// Copy these records and counts from the live graph.
    Records(RegionGraphUndoDelta),
    /// Clone the view's graph: after a load, a clear or a full rebuild.
    Replace,
}

/// One publication of the road tool's view, in order.
pub(crate) struct RoadToolPublication {
    pub(crate) sequence: u64,
    pub(crate) view: RoadToolQuerySnapshot,
    pub(crate) replica: ReplicaSync,
    // The view the simulation published before, which it no longer holds; freed with the
    // context's and the shared view's predecessors off the publishing thread.
    retired: Option<RoadToolQuerySnapshot>,
}

// What one publication replaced: the previous views and, after a full replacement, the old
// replica. They may hold the last copies of a graph, a heightmap and a watermap, whose frees
// are `O(city)` and decide nothing, so `retire` frees them on the shared pool instead of on
// the simulation thread under the road tool's locks (`ROAD-68-E`).
struct RetiredRoadToolState {
    views: [Option<RoadToolQuerySnapshot>; 3],
    replica: Option<RegionGraph>,
}

impl RetiredRoadToolState {
    // Frees everything on the shared pool without waiting.
    fn retire(self) {
        if self.views.iter().any(Option::is_some) || self.replica.is_some() {
            crate::thread_pools::spawn(move || drop(self));
        }
    }
}

impl SimCore {
    /// Publishes the road tool's view of the current generation once the roadbed matches the
    /// graph; `None` while it is dirty (the rebuilt owners keep accumulating).
    ///
    /// `graph` is an immutable copy of the live graph the caller already made (the commit's
    /// routing rebuild input), which the view shares; without one the view keeps its graph when
    /// nothing changed and copies the live graph otherwise. Terrain and water are copied only
    /// when their generations changed. Cost: `O(rebuilt owners' records)` plus those copies.
    pub(crate) fn publish_road_tool_view(
        &mut self,
        graph: Option<Arc<RegionGraph>>,
    ) -> Option<RoadToolPublication> {
        let graph = graph.or_else(|| self.road_tool_publisher.offered_graph.take());
        self.road_tool_publisher.offered_graph = None;
        if !self.transit_network.roadbed.is_current() {
            return None;
        }
        let terrain_inputs = self.refresh_road_terrain_inputs();
        #[cfg(debug_assertions)]
        if let Some(graph) = &graph {
            assert_replica_matches(graph, &self.region_graph);
        }
        let rebuilt = self.transit_network.roadbed.take_rebuilt_owners();
        let terrain_generations = (
            self.heightmap.source_generation(),
            self.heightmap.visual_generation(),
        );
        let water_generation = self.watermap.render_generation();
        let graph_counts = (
            self.region_graph.node_count(),
            self.region_graph.edge_count(),
        );
        let role_revision = self.region_graph.node_role_revision();
        let previous = self.road_tool_publisher.published.take();
        let (view, replica) = match &previous {
            Some(previous) if !rebuilt.all && previous.role_revision == role_revision => {
                let graph_changed = !rebuilt.is_empty() || previous.graph_counts != graph_counts;
                let replica = if graph_changed {
                    let nodes = rebuilt.nodes.iter().copied().collect();
                    let edges = rebuilt.edges.iter().copied().collect();
                    let records = self
                        .region_graph
                        .capture_undo_for_local_topology(edges, nodes);
                    ReplicaSync::Records(records)
                } else {
                    ReplicaSync::Unchanged
                };
                let region_graph = match graph {
                    Some(graph) => graph,
                    None if graph_changed => Arc::new(self.region_graph.clone()),
                    None => Arc::clone(&previous.view.region_graph),
                };
                let terrain = if previous.terrain_generations == terrain_generations {
                    Arc::clone(&previous.view.terrain)
                } else {
                    Arc::new(self.heightmap.clone())
                };
                let water = if previous.water_generation == water_generation {
                    Arc::clone(&previous.view.water)
                } else {
                    Arc::new(self.watermap.clone())
                };
                let view = RoadToolQuerySnapshot {
                    zoning_grid: self
                        .zoning
                        .cells
                        .road_grid_snap(f64::from(self.config.zone_cell_m)),
                    terrain,
                    region_graph,
                    roadbed: Arc::new(self.transit_network.roadbed.snapshot()),
                    water,
                    road_mesh_chunks: Arc::clone(&self.published_road_mesh_chunks),
                    road_mesh_generation: self.road_mesh_lane_generation,
                    terrain_inputs,
                    fields: Arc::clone(&self.allocator.field_clearance),
                    cell_reservations: self.zoning.cells.reservations().clone(),
                    surface_generation: self.road_tool_surface_generation,
                };
                (view, replica)
            }
            _ => {
                let mut view = road_tool_snapshot_from_core(self)?;
                if let Some(graph) = graph {
                    view.region_graph = graph;
                }
                (view, ReplicaSync::Replace)
            }
        };
        self.road_tool_publisher.sequence += 1;
        self.road_tool_publisher.published = Some(PublishedView {
            view: view.clone(),
            terrain_generations,
            water_generation,
            graph_counts,
            role_revision,
        });
        Some(RoadToolPublication {
            sequence: self.road_tool_publisher.sequence,
            view,
            replica,
            retired: previous.map(|previous| previous.view),
        })
    }
}

/// The road tool's shared state on the Godot side: the view that cursor, hover and terrain jobs
/// read, and the preview's context. Publications arrive from the simulation thread and from
/// Godot-thread edits; each takes effect only if it is newer than the context's.
pub(crate) struct RoadToolShared {
    pub(crate) view: RwLock<RoadToolQuerySnapshot>,
    pub(crate) context: Mutex<RoadPreviewContext>,
}

impl RoadToolShared {
    /// Captures the shared state from a first view (one graph copy for the replica).
    #[cfg(test)]
    pub(crate) fn new(view: RoadToolQuerySnapshot) -> Self {
        Self {
            context: Mutex::new(RoadPreviewContext::capture(view.clone())),
            view: RwLock::new(view),
        }
    }

    /// Captures the shared state from a publication, keeping its sequence so the next
    /// publication follows by records instead of replacing the replica.
    pub(crate) fn from_publication(publication: RoadToolPublication) -> Self {
        let mut context = RoadPreviewContext::capture(publication.view.clone());
        context.sequence = publication.sequence;
        Self {
            context: Mutex::new(context),
            view: RwLock::new(publication.view),
        }
    }

    /// The latest preview when it was made for exactly this stroke.
    pub(crate) fn preview_for(
        &self,
        points: &[godot::prelude::Vector3],
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
    ) -> Option<Arc<RoadPreview>> {
        let context = self
            .context
            .lock()
            .expect("road tool context lock poisoned");
        context
            .last_preview
            .as_ref()
            .filter(|preview| preview.is_for(points, fwd_lanes, bkw_lanes, snap_to_existing_roads))
            .cloned()
    }

    /// Applies a publication to the context and, when it was newer, to the view. Terrain inputs
    /// newer than the publication's (published since by [`Self::publish_terrain_inputs`]) stay.
    /// The replaced views are swapped out under the locks and freed on the shared pool.
    pub(crate) fn publish(&self, mut publication: RoadToolPublication) {
        let mut retired = RetiredRoadToolState {
            views: [publication.retired.take(), None, None],
            replica: None,
        };
        let mut context = self
            .context
            .lock()
            .expect("road tool context lock poisoned");
        if let Some((view, replica)) = context.follow(&publication) {
            publication.view.terrain_inputs = context.view.terrain_inputs.clone();
            let shared = std::mem::replace(
                &mut *self.view.write().expect("road tool view lock poisoned"),
                publication.view,
            );
            retired.views[1] = Some(view);
            retired.views[2] = Some(shared);
            retired.replica = replica;
        } else {
            retired.views[1] = Some(publication.view);
        }
        drop(context);
        retired.retire();
    }

    /// Takes newer terrain inputs into the context and the view, between publications: the
    /// Godot thread publishes them after inserting refined entries its payload jobs built, so
    /// the next preview reuses them as the commit will (`ROAD-68-E`).
    pub(crate) fn publish_terrain_inputs(&self, inputs: RoadTerrainInputs) {
        let mut context = self
            .context
            .lock()
            .expect("road tool context lock poisoned");
        if inputs.revision <= context.view.terrain_inputs.revision {
            return;
        }
        context.view.terrain_inputs = inputs.clone();
        self.view
            .write()
            .expect("road tool view lock poisoned")
            .terrain_inputs = inputs;
    }
}

/// The road tool's private, mutable copy of the published graph, plus the view it belongs to.
/// A preview applies its plan to the replica, compiles, and restores it (`ROAD-68-E`).
pub(crate) struct RoadPreviewContext {
    pub(crate) view: RoadToolQuerySnapshot,
    pub(crate) replica: RegionGraph,
    sequence: u64,
    // The latest preview; a click on the same stroke submits it with the commit.
    pub(crate) last_preview: Option<Arc<RoadPreview>>,
    // The roadbed builders' buffers, reused by every pose's compile (guardrail: per-pose
    // scratch lives in the preview context).
    pub(crate) roadbed_scratch: crate::simulation::network::roadbed::RoadbedScratch,
}

impl RoadPreviewContext {
    /// Captures a context from a view, copying its graph once: the activation capture, measured
    /// separately from any pose (`O(city)`).
    pub(crate) fn capture(view: RoadToolQuerySnapshot) -> Self {
        Self {
            replica: (*view.region_graph).clone(),
            view,
            sequence: 0,
            last_preview: None,
            roadbed_scratch: Default::default(),
        }
    }

    /// Follows one publication, as [`Self::follow`] does, and frees what it replaced in place.
    #[cfg(test)]
    pub(crate) fn apply(&mut self, publication: &RoadToolPublication) -> bool {
        self.follow(publication).is_some()
    }

    /// Follows one publication and returns the view it replaced, with the old replica when the
    /// publication replaced it; `None` when the publication is older than the context, which
    /// ignores it. A publication that skips one replaces the replica from its view.
    pub(crate) fn follow(
        &mut self,
        publication: &RoadToolPublication,
    ) -> Option<(RoadToolQuerySnapshot, Option<RegionGraph>)> {
        if publication.sequence <= self.sequence {
            return None;
        }
        let in_order = publication.sequence == self.sequence + 1;
        let replaced_replica = match &publication.replica {
            ReplicaSync::Unchanged if in_order => None,
            ReplicaSync::Records(records) if in_order => {
                self.replica.restore_undo_delta_local(records);
                None
            }
            _ => Some(std::mem::replace(
                &mut self.replica,
                (*publication.view.region_graph).clone(),
            )),
        };
        let newer_terrain = (self.view.terrain_inputs.revision
            > publication.view.terrain_inputs.revision)
            .then(|| self.view.terrain_inputs.clone());
        let replaced = std::mem::replace(&mut self.view, publication.view.clone());
        if let Some(inputs) = newer_terrain {
            self.view.terrain_inputs = inputs;
        }
        self.sequence = publication.sequence;
        self.last_preview = None;
        #[cfg(debug_assertions)]
        assert_replica_matches(&self.replica, &self.view.region_graph);
        Some((replaced, replaced_replica))
    }
}

// Debug builds check that the replica equals the published graph in every value the road
// tool's plans and pieces read. `O(city)`, like the demand pass's book fingerprints.
#[cfg(debug_assertions)]
fn assert_replica_matches(replica: &RegionGraph, published: &RegionGraph) {
    assert_eq!(
        replica.node_count(),
        published.node_count(),
        "replica nodes"
    );
    assert_eq!(
        replica.edge_count(),
        published.edge_count(),
        "replica edges"
    );
    assert_eq!(
        replica.node_adjacency_count(),
        published.node_adjacency_count(),
        "replica adjacency"
    );
    for node in 0..replica.node_adjacency_count().min(replica.node_count()) as u32 {
        let (a, b) = (replica.node(node), published.node(node));
        assert!(
            a.pos == b.pos
                && a.node_type == b.node_type
                && a.crosswalk_overrides == b.crosswalk_overrides
                && replica.get_valid_node(node) == published.get_valid_node(node)
                && replica.node_adjacency(node) == published.node_adjacency(node),
            "replica node {node} differs from the published graph"
        );
    }
    for edge in 0..replica.edge_count() {
        let (a, b) = (replica.edge(edge), published.edge(edge));
        if a.deleted && b.deleted {
            continue;
        }
        assert!(
            a.start_node == b.start_node
                && a.end_node == b.end_node
                && a.primary_type == b.primary_type
                && a.allowed_types == b.allowed_types
                && a.class == b.class
                && a.width == b.width
                && a.fwd_lanes == b.fwd_lanes
                && a.bkw_lanes == b.bkw_lanes
                && a.start_clip == b.start_clip
                && a.end_clip == b.end_clip
                && a.geometry == b.geometry
                && a.physical_geometry == b.physical_geometry
                && a.deleted == b.deleted,
            "replica edge {edge} differs from the published graph"
        );
    }
}
