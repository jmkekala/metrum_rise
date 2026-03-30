//! Main simulation loop for agents: transit state machine and movement.

use super::data::AgentSystem;
use super::{
    MODE_CAR, MODE_WALK, TRANSIT_ARRIVING, TRANSIT_DEPARTING, TRANSIT_IDLE, TRANSIT_IMMIGRATING,
    TRANSIT_INTERSECTION, TRANSIT_ON_ROAD,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::network::TransitNetwork;
use crate::simulation::pathing::flow_field::FlowField;
use godot::prelude::*;
use rand::Rng;
use rayon::prelude::*;
use std::cell::RefCell;
use std::sync::atomic::Ordering;

// ---------------------------------------------------------------------------
// Thread-local scratch buffers — pre-allocated per Rayon worker thread to
// avoid any heap allocation in the per-agent hot path.
// ---------------------------------------------------------------------------
thread_local! {
    static VALID_LANES: RefCell<Vec<usize>> = RefCell::new(Vec::with_capacity(8));
    static VALID_CONNS: RefCell<Vec<usize>> = RefCell::new(Vec::with_capacity(8));
}

// ---------------------------------------------------------------------------
// Unsafe raw-slice wrapper.
//
// Safety invariant upheld throughout this module:
//   Rayon's `(0..n).into_par_iter()` guarantees that each index `i` is
//   visited by exactly one thread at a time.  All mutable field accesses
//   below index into disjoint locations, so there is no data race.
//   The wrapper is `Send + Sync` only within this module; it is never
//   stored beyond the lifetime of the parallel scope.
// ---------------------------------------------------------------------------
struct RawSlice<T> {
    ptr: *mut T,
    len: usize,
}
unsafe impl<T: Send> Send for RawSlice<T> {}
unsafe impl<T: Send> Sync for RawSlice<T> {}

impl<T> RawSlice<T> {
    fn new(v: &mut Vec<T>) -> Self {
        Self { ptr: v.as_mut_ptr(), len: v.len() }
    }
    #[inline(always)]
    unsafe fn get(&self, i: usize) -> &T {
        debug_assert!(i < self.len);
        unsafe { &*self.ptr.add(i) }
    }
    #[inline(always)]
    unsafe fn get_mut(&self, i: usize) -> &mut T {
        debug_assert!(i < self.len);
        unsafe { &mut *self.ptr.add(i) }
    }
}

impl AgentSystem {
    /// Advances the agent simulation by `delta` seconds.
    pub fn tick(
        &mut self,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        delta: f32,
    ) {
        self.sim_time += delta;
        let n = self.agents.len();
        if n == 0 {
            return;
        }

        let bldg_count = allocator.buildings.len();

        // -----------------------------------------------------------------------
        // 1. Safety Scrub — parallel, each agent is independent.
        // -----------------------------------------------------------------------
        let s_home     = RawSlice::new(&mut self.agents.home_building);
        let s_work     = RawSlice::new(&mut self.agents.work_building);
        let s_cur_b    = RawSlice::new(&mut self.agents.current_building);
        let s_tgt_b    = RawSlice::new(&mut self.agents.target_building);
        let s_transit  = RawSlice::new(&mut self.agents.transit);
        let s_visible  = RawSlice::new(&mut self.agents.is_visible);

        (0..n).into_par_iter().for_each(|i| {
            unsafe {
                if *s_home.get(i) != usize::MAX && *s_home.get(i) >= bldg_count {
                    *s_home.get_mut(i) = usize::MAX;
                }
                if *s_work.get(i) != usize::MAX && *s_work.get(i) >= bldg_count {
                    *s_work.get_mut(i) = usize::MAX;
                }
                if *s_cur_b.get(i) != usize::MAX && *s_cur_b.get(i) >= bldg_count {
                    *s_cur_b.get_mut(i) = usize::MAX;
                    *s_transit.get_mut(i) = TRANSIT_ARRIVING;
                    *s_visible.get_mut(i) = true;
                }
                let tgt = *s_tgt_b.get(i);
                if tgt != usize::MAX && tgt >= bldg_count {
                    let home = *s_home.get(i);
                    if home != usize::MAX {
                        *s_tgt_b.get_mut(i) = home;
                    } else {
                        *s_tgt_b.get_mut(i) = usize::MAX;
                        *s_transit.get_mut(i) = TRANSIT_ARRIVING;
                    }
                }
            }
        });

        // -----------------------------------------------------------------------
        // 2. Main agent loop — parallel.
        //
        // All shared inputs (allocator, transit_network, graph) are read-only.
        // All writes go to disjoint per-agent SoA slots.
        // pathfind_count uses AtomicU32::fetch_add (relaxed).
        // -----------------------------------------------------------------------
        let s_home      = RawSlice::new(&mut self.agents.home_building);
        let s_work      = RawSlice::new(&mut self.agents.work_building);
        let s_pos_x     = RawSlice::new(&mut self.agents.pos_x);
        let s_pos_y     = RawSlice::new(&mut self.agents.pos_y);
        let s_visible   = RawSlice::new(&mut self.agents.is_visible);
        let s_activity  = RawSlice::new(&mut self.agents.activity);
        let s_transit   = RawSlice::new(&mut self.agents.transit);
        let s_happiness = RawSlice::new(&mut self.agents.happiness);
        let s_money     = RawSlice::new(&mut self.agents.money);
        let s_jstart    = RawSlice::new(&mut self.agents.journey_start_time);
        let s_cur_b     = RawSlice::new(&mut self.agents.current_building);
        let s_tgt_b     = RawSlice::new(&mut self.agents.target_building);
        let s_cur_n     = RawSlice::new(&mut self.agents.current_node);
        let s_tgt_n     = RawSlice::new(&mut self.agents.target_node);
        let s_cur_e     = RawSlice::new(&mut self.agents.current_edge);
        let s_lane_id   = RawSlice::new(&mut self.agents.current_lane_id);
        let s_lane_d    = RawSlice::new(&mut self.agents.lane_distance);
        let s_tmode     = RawSlice::new(&mut self.agents.transit_mode);
        let s_path      = RawSlice::new(&mut self.agents.current_path);
        let s_path_idx  = RawSlice::new(&mut self.agents.current_path_index);
        let s_has_car   = RawSlice::new(&mut self.agents.has_car);
        let _s_vtype    = RawSlice::new(&mut self.agents.vehicle_type);

        let pathfind_count = &self.pathfind_count;
        let sim_time = self.sim_time;

        (0..n).into_par_iter().for_each(|i| {
            let mut rng = rand::thread_rng();

            // Safety: index i is unique to this thread via par_iter.
            unsafe {
                *s_cur_n.get_mut(i) = graph.get_valid_node(*s_cur_n.get(i));
                *s_tgt_n.get_mut(i) = graph.get_valid_node(*s_tgt_n.get(i));

                match *s_transit.get(i) {
                    TRANSIT_IDLE => {
                        if rng.gen_bool((0.05 * delta) as f64) {
                            let mut next_act = *s_activity.get(i);
                            let mut next_bldg = usize::MAX;

                            if *s_activity.get(i) == 0 {
                                if *s_money.get(i) >= 20.0 && rng.gen_bool(0.4) {
                                    if let Some(h) = allocator.get_random_building_by_zone(
                                        ZoneType::Commercial, &mut rng,
                                    ) {
                                        next_bldg = h;
                                        next_act = 2;
                                    }
                                } else {
                                    if *s_work.get(i) == usize::MAX {
                                        if let Some(h) = allocator.get_random_building_by_zones(
                                            &[ZoneType::Industrial, ZoneType::Commercial],
                                            &mut rng,
                                        ) {
                                            *s_work.get_mut(i) = h;
                                        }
                                    }
                                    if *s_work.get(i) != usize::MAX {
                                        next_bldg = *s_work.get(i);
                                        next_act = 1;
                                    }
                                }
                            } else if *s_home.get(i) != usize::MAX {
                                next_bldg = *s_home.get(i);
                                next_act = 0;
                            }

                            let curr_bldg = *s_cur_b.get(i);
                            if next_bldg != usize::MAX
                                && next_bldg < allocator.buildings.len()
                                && curr_bldg != usize::MAX
                                && curr_bldg < allocator.buildings.len()
                            {
                                let origin_node = allocator.buildings[curr_bldg].frontage_node;
                                let target_node = allocator.buildings[next_bldg].frontage_node;
                                *s_cur_n.get_mut(i) = origin_node;
                                *s_tgt_n.get_mut(i) = target_node;

                                let has_car = *s_has_car.get(i);
                                let mode = if has_car { MODE_CAR } else { MODE_WALK };
                                let search_flags = if has_car { TransitFlags::CAR } else { TransitFlags::FOOT };

                                // Try flow field first (O(path_len) chain walk, no Dijkstra).
                                // Flow fields route to the nearest building of the target zone;
                                // they are only used for work/shop trips (next_act != 0).
                                // Home trips (next_act == 0) use CCH since home is a specific building.
                                let target_zone = if next_act == 1 {
                                    Some(allocator.buildings[next_bldg].zone_type)
                                } else {
                                    None // shop or home — flow field valid for shop too
                                };
                                let ff: Option<&FlowField> = target_zone.and_then(|z| {
                                    if has_car {
                                        transit_network.flow_fields.car(z)
                                    } else {
                                        transit_network.flow_fields.foot(z)
                                    }
                                });

                                let path_opt: Option<Vec<u32>> = ff
                                    .and_then(|f| f.build_path(origin_node, graph.nodes.len() + 1))
                                    .or_else(|| {
                                        // Fall back to CCH for home trips, novel destinations,
                                        // or when flow field is not yet built.
                                        pathfind_count.fetch_add(1, Ordering::Relaxed);
                                        transit_network.cch_graph
                                            .find_path(origin_node, target_node, usize::MAX, graph, search_flags)
                                            .map(|(_, _, p)| p)
                                    });

                                if let Some(path) = path_opt {
                                    // Update target_building to match where the flow field actually leads.
                                    let effective_tgt = if let Some(f) = ff {
                                        let dest_node = path.last().copied().unwrap_or(target_node);
                                        let nearest = f.nearest_building[origin_node as usize];
                                        if nearest != usize::MAX
                                            && allocator.buildings[nearest].frontage_node == dest_node
                                        {
                                            nearest
                                        } else {
                                            next_bldg
                                        }
                                    } else {
                                        next_bldg
                                    };

                                    if path.len() > 1 {
                                        *s_tgt_b.get_mut(i) = effective_tgt;
                                        *s_activity.get_mut(i) = next_act;
                                        *s_jstart.get_mut(i) = sim_time;
                                        *s_tmode.get_mut(i) = mode;
                                        *s_path.get_mut(i) = path;
                                        *s_path_idx.get_mut(i) = 1;
                                        *s_lane_id.get_mut(i) = usize::MAX;
                                        *s_lane_d.get_mut(i) = 0.0;
                                        *s_visible.get_mut(i) = true;
                                        *s_transit.get_mut(i) = TRANSIT_DEPARTING;
                                    } else if origin_node == target_node {
                                        *s_tgt_b.get_mut(i) = effective_tgt;
                                        *s_activity.get_mut(i) = next_act;
                                        *s_jstart.get_mut(i) = sim_time;
                                        *s_tmode.get_mut(i) = MODE_WALK;
                                        s_path.get_mut(i).clear();
                                        *s_path_idx.get_mut(i) = 0;
                                        *s_lane_id.get_mut(i) = usize::MAX;
                                        *s_lane_d.get_mut(i) = 0.0;
                                        *s_visible.get_mut(i) = true;
                                        *s_transit.get_mut(i) = TRANSIT_ARRIVING;
                                    }
                                }
                            }
                        }
                    }

                    TRANSIT_DEPARTING => {
                        let b_id = *s_cur_b.get(i);
                        if b_id == usize::MAX || b_id >= allocator.buildings.len() {
                            *s_transit.get_mut(i) = TRANSIT_ON_ROAD;
                            return;
                        }
                        let frontage_node = allocator.buildings[b_id].frontage_node;
                        if frontage_node as usize >= graph.nodes.len() {
                            *s_transit.get_mut(i) = TRANSIT_IDLE;
                            *s_visible.get_mut(i) = false;
                            return;
                        }
                        let node_pos = graph.nodes[frontage_node as usize].pos;
                        let target_vec = Vector2::new(node_pos.x, node_pos.z);
                        let dir = target_vec - Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i));
                        let dist = dir.length();
                        let speed = if *s_tmode.get(i) == MODE_CAR { 10.0 } else { 4.0 };
                        let step = speed * delta;
                        if dist < step {
                            *s_pos_x.get_mut(i) = target_vec.x;
                            *s_pos_y.get_mut(i) = target_vec.y;
                            *s_cur_n.get_mut(i) = frontage_node;
                            *s_cur_e.get_mut(i) = usize::MAX;
                            *s_lane_id.get_mut(i) = usize::MAX;
                            *s_lane_d.get_mut(i) = 0.0;
                            *s_transit.get_mut(i) = TRANSIT_ON_ROAD;
                        } else {
                            let mv = dir.normalized() * step;
                            *s_pos_x.get_mut(i) += mv.x;
                            *s_pos_y.get_mut(i) += mv.y;
                        }
                    }

                    TRANSIT_ON_ROAD | TRANSIT_IMMIGRATING | TRANSIT_INTERSECTION => {
                        let speed = if *s_tmode.get(i) == MODE_CAR {
                            if *s_transit.get(i) == TRANSIT_INTERSECTION { 10.0 } else { 20.0 }
                        } else {
                            4.0
                        };
                        let mut remaining_dist = speed * delta;

                        while remaining_dist > 0.0 {
                            // 1. Init path if missing — try flow field first, fall back to CCH.
                            if s_path.get(i).is_empty() {
                                let cur_n = *s_cur_n.get(i);
                                let tgt_n = *s_tgt_n.get(i);
                                let is_walk = *s_tmode.get(i) == MODE_WALK;
                                let search_flags = if is_walk { TransitFlags::FOOT } else { TransitFlags::CAR };

                                // Try flow field: look up by target building's zone type.
                                let t_bldg = *s_tgt_b.get(i);
                                let ff: Option<&FlowField> = if t_bldg != usize::MAX
                                    && t_bldg < allocator.buildings.len()
                                {
                                    let zone = allocator.buildings[t_bldg].zone_type;
                                    if is_walk {
                                        transit_network.flow_fields.foot(zone)
                                    } else {
                                        transit_network.flow_fields.car(zone)
                                    }
                                } else {
                                    None
                                };

                                let path_opt: Option<Vec<u32>> = ff
                                    .and_then(|f| f.build_path(cur_n, graph.nodes.len() + 1))
                                    .filter(|p| p.len() > 1)
                                    .or_else(|| {
                                        pathfind_count.fetch_add(1, Ordering::Relaxed);
                                        transit_network.cch_graph
                                            .find_path(cur_n, tgt_n, usize::MAX, graph, search_flags)
                                            .and_then(|(_, _, p)| if p.len() > 1 { Some(p) } else { None })
                                    });

                                if let Some(path) = path_opt {
                                    *s_path.get_mut(i) = path;
                                    *s_path_idx.get_mut(i) = 1;
                                    *s_lane_id.get_mut(i) = usize::MAX;
                                } else {
                                    *s_transit.get_mut(i) = TRANSIT_IDLE;
                                    *s_visible.get_mut(i) = false;
                                    break;
                                }
                            }

                            // 2. Init lane if entering network
                            if *s_lane_id.get(i) == usize::MAX {
                                let path = s_path.get(i);
                                let idx = *s_path_idx.get(i);
                                if idx < path.len() {
                                    let next_node = path[idx];
                                    if let Some(best_e) = graph.get_edge_between_nodes(*s_cur_n.get(i), next_node) {
                                        let edge = &graph.edges[best_e];
                                        let is_fwd = edge.start_node == *s_cur_n.get(i);
                                        if let Some(edge_lanes) = transit_network.lane_system.edge_lanes.get(&best_e) {
                                            VALID_LANES.with(|v| {
                                                let mut valid_lanes = v.borrow_mut();
                                                valid_lanes.clear();
                                                for &l_id in edge_lanes {
                                                    let lane = &transit_network.lane_system.lanes[l_id];
                                                    if lane.is_fwd == is_fwd {
                                                        if *s_tmode.get(i) == MODE_WALK {
                                                            if lane.lane_type == crate::simulation::network::lanes::LaneType::Foot {
                                                                let b_idx = *s_cur_b.get(i);
                                                                if b_idx != usize::MAX && b_idx < allocator.buildings.len() {
                                                                    let b_side = allocator.buildings[b_idx].side;
                                                                    let lane_side = if lane.lane_idx > 0 { 1 } else { -1 };
                                                                    if lane_side == b_side {
                                                                        valid_lanes.push(l_id);
                                                                    }
                                                                } else {
                                                                    valid_lanes.push(l_id);
                                                                }
                                                            }
                                                        } else if lane.lane_type == crate::simulation::network::lanes::LaneType::Vehicle {
                                                            valid_lanes.push(l_id);
                                                        }
                                                    }
                                                }
                                                if !valid_lanes.is_empty() {
                                                    let chosen = valid_lanes[rng.gen_range(0..valid_lanes.len())];
                                                    *s_lane_id.get_mut(i) = chosen;
                                                    *s_lane_d.get_mut(i) = 0.0;
                                                    *s_cur_e.get_mut(i) = best_e;
                                                    *s_transit.get_mut(i) = TRANSIT_ON_ROAD;
                                                    *s_cur_b.get_mut(i) = usize::MAX;
                                                } else {
                                                    s_path.get_mut(i).clear();
                                                }
                                            });
                                            if s_path.get(i).is_empty() { break; }
                                        } else {
                                            s_path.get_mut(i).clear();
                                            break;
                                        }
                                    } else {
                                        s_path.get_mut(i).clear();
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            }

                            // 3. Movement along lane
                            let lane_id = *s_lane_id.get(i);
                            if lane_id >= transit_network.lane_system.lanes.len() {
                                *s_lane_id.get_mut(i) = usize::MAX;
                                s_path.get_mut(i).clear();
                                break;
                            }

                            let lane = &transit_network.lane_system.lanes[lane_id];
                            let dist_to_end = lane.length - *s_lane_d.get(i);

                            if remaining_dist < dist_to_end {
                                *s_lane_d.get_mut(i) += remaining_dist;
                                remaining_dist = 0.0;

                                // Midway arrival check
                                let t_bldg_idx = *s_tgt_b.get(i);
                                if t_bldg_idx != usize::MAX && t_bldg_idx < allocator.buildings.len() {
                                    let b = &allocator.buildings[t_bldg_idx];
                                    if lane.edge_id == b.edge_idx {
                                        let tgt_len = graph.edges[b.edge_idx].physical_length;
                                        let progress_ratio = *s_lane_d.get(i) / lane.length.max(0.001);
                                        let agent_prog = if lane.is_fwd {
                                            progress_ratio * tgt_len
                                        } else {
                                            (1.0 - progress_ratio) * tgt_len
                                        };
                                        let side_matches = *s_tmode.get(i) != MODE_WALK
                                            || lane.lane_idx == (b.side as i8) * 100
                                            || lane.lane_idx == 0;
                                        if side_matches && (agent_prog - (b.frontage_t * tgt_len)).abs() < 4.0 {
                                            *s_transit.get_mut(i) = TRANSIT_ARRIVING;
                                            *s_tmode.get_mut(i) = MODE_WALK;
                                            *s_lane_id.get_mut(i) = usize::MAX;
                                            s_path.get_mut(i).clear();
                                            remaining_dist = 0.0;
                                        }
                                    }
                                }
                            } else {
                                // Reached end of lane
                                remaining_dist -= dist_to_end;

                                if lane.edge_id != usize::MAX {
                                    *s_cur_n.get_mut(i) = if lane.is_fwd {
                                        graph.edges[lane.edge_id].end_node
                                    } else {
                                        graph.edges[lane.edge_id].start_node
                                    };

                                    *s_path_idx.get_mut(i) += 1;
                                    let path_idx = *s_path_idx.get(i);
                                    let path_len = s_path.get(i).len();

                                    if path_idx < path_len {
                                        let next_node = s_path.get(i)[path_idx];
                                        if let Some(best_e) = graph.get_edge_between_nodes(*s_cur_n.get(i), next_node) {
                                            VALID_CONNS.with(|v| {
                                                let mut valid_conns = v.borrow_mut();
                                                valid_conns.clear();
                                                for &c_id in &lane.next_lanes {
                                                    if c_id < transit_network.lane_system.lanes.len() {
                                                        let conn_lane = &transit_network.lane_system.lanes[c_id];
                                                        if !conn_lane.next_lanes.is_empty() {
                                                            let tgt_road_lane = conn_lane.next_lanes[0];
                                                            if tgt_road_lane < transit_network.lane_system.lanes.len()
                                                                && transit_network.lane_system.lanes[tgt_road_lane].edge_id == best_e
                                                            {
                                                                valid_conns.push(c_id);
                                                            }
                                                        }
                                                    }
                                                }
                                                if !valid_conns.is_empty() {
                                                    *s_lane_id.get_mut(i) = valid_conns[rng.gen_range(0..valid_conns.len())];
                                                    *s_lane_d.get_mut(i) = 0.0;
                                                    *s_transit.get_mut(i) = TRANSIT_INTERSECTION;
                                                    *s_cur_e.get_mut(i) = usize::MAX;
                                                } else {
                                                    s_path.get_mut(i).clear();
                                                    *s_lane_id.get_mut(i) = usize::MAX;
                                                }
                                            });
                                            if s_path.get(i).is_empty() { break; }
                                        } else {
                                            s_path.get_mut(i).clear();
                                            *s_lane_id.get_mut(i) = usize::MAX;
                                            break;
                                        }
                                    } else {
                                        s_path.get_mut(i).clear();
                                        *s_lane_id.get_mut(i) = usize::MAX;
                                        let t_bldg = *s_tgt_b.get(i);
                                        if t_bldg != usize::MAX && t_bldg < allocator.buildings.len() {
                                            *s_transit.get_mut(i) = TRANSIT_ARRIVING;
                                            *s_tmode.get_mut(i) = MODE_WALK;
                                        }
                                        break;
                                    }
                                } else {
                                    // Intersection connection — move to next road lane
                                    if !lane.next_lanes.is_empty() {
                                        let tgt_road_lane = lane.next_lanes[0];
                                        if tgt_road_lane < transit_network.lane_system.lanes.len() {
                                            *s_lane_id.get_mut(i) = tgt_road_lane;
                                            *s_lane_d.get_mut(i) = 0.0;
                                            *s_transit.get_mut(i) = TRANSIT_ON_ROAD;
                                            *s_cur_e.get_mut(i) = transit_network.lane_system.lanes[tgt_road_lane].edge_id;
                                        } else {
                                            s_path.get_mut(i).clear();
                                            *s_lane_id.get_mut(i) = usize::MAX;
                                            break;
                                        }
                                    } else {
                                        s_path.get_mut(i).clear();
                                        *s_lane_id.get_mut(i) = usize::MAX;
                                        break;
                                    }
                                }
                            }
                        }

                        // 4. Position output from lane
                        let current_lane = *s_lane_id.get(i);
                        if current_lane != usize::MAX && current_lane < transit_network.lane_system.lanes.len() {
                            let l = &transit_network.lane_system.lanes[current_lane];
                            let dist = *s_lane_d.get(i);
                            if dist <= 0.0 && !l.geometry.is_empty() {
                                *s_pos_x.get_mut(i) = l.geometry[0].x;
                                *s_pos_y.get_mut(i) = l.geometry[0].z;
                            } else if dist >= l.length && !l.geometry.is_empty() {
                                let end = l.geometry.last().unwrap();
                                *s_pos_x.get_mut(i) = end.x;
                                *s_pos_y.get_mut(i) = end.z;
                            } else if l.geometry.len() >= 2 && !l.cum_dist.is_empty() {
                                let seg = l.cum_dist.partition_point(|&d| d <= dist).saturating_sub(1);
                                let seg = seg.min(l.geometry.len() - 2);
                                let p0 = l.geometry[seg];
                                let p1 = l.geometry[seg + 1];
                                let seg_len = l.cum_dist[seg + 1] - l.cum_dist[seg];
                                let t = if seg_len > 1e-5 { (dist - l.cum_dist[seg]) / seg_len } else { 0.0 };
                                let mut out = p0.lerp(p1, t.clamp(0.0, 1.0));
                                if *s_tmode.get(i) == MODE_WALK && seg_len > 1e-5 {
                                    let tangent = (p1 - p0) / seg_len;
                                    let normal = Vector3::new(-tangent.z, 0.0, tangent.x);
                                    let jitter = (f32::sin(i as f32 * 4.0) + f32::cos(i as f32 * 7.0)) * 0.7;
                                    out += normal * jitter;
                                }
                                *s_pos_x.get_mut(i) = out.x;
                                *s_pos_y.get_mut(i) = out.z;
                            }
                        }
                    }

                    TRANSIT_ARRIVING => {
                        let b_id = *s_tgt_b.get(i);
                        if b_id == usize::MAX {
                            *s_transit.get_mut(i) = TRANSIT_IDLE;
                            return;
                        }
                        if b_id >= allocator.buildings.len() {
                            *s_transit.get_mut(i) = TRANSIT_IDLE;
                            return;
                        }
                        let b = &allocator.buildings[b_id];
                        let center_vec = Vector2::new(b.center_x, b.center_y);
                        let dir_to_center = center_vec - Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i));
                        let dist = dir_to_center.length();
                        let speed = if *s_tmode.get(i) == MODE_CAR { 10.0 } else { 4.0 };
                        let step = speed * delta;

                        if dist < step {
                            let prev_activity = *s_activity.get(i);
                            *s_pos_x.get_mut(i) = center_vec.x;
                            *s_pos_y.get_mut(i) = center_vec.y;
                            *s_cur_b.get_mut(i) = b_id;
                            *s_visible.get_mut(i) = false;
                            *s_transit.get_mut(i) = TRANSIT_IDLE;
                            let home = *s_home.get(i);
                            let work = *s_work.get(i);
                            if b_id == home {
                                *s_activity.get_mut(i) = 0;
                            } else if b_id == work {
                                *s_activity.get_mut(i) = 1;
                            } else {
                                *s_activity.get_mut(i) = 2;
                            }
                            *s_tmode.get_mut(i) = MODE_WALK;
                            *s_cur_e.get_mut(i) = usize::MAX;
                            *s_lane_id.get_mut(i) = usize::MAX;
                            *s_lane_d.get_mut(i) = 0.0;
                            s_path.get_mut(i).clear();
                            *s_path_idx.get_mut(i) = 0;

                            let commute_time = sim_time - *s_jstart.get(i);
                            *s_happiness.get_mut(i) =
                                (*s_happiness.get(i) - commute_time / 60.0).clamp(0.0, 100.0);
                            if prev_activity == 2 {
                                *s_money.get_mut(i) = (*s_money.get(i) - 20.0).max(0.0);
                            }
                        } else if dist > 0.0001 {
                            let mv = dir_to_center.normalized() * step;
                            *s_pos_x.get_mut(i) += mv.x;
                            *s_pos_y.get_mut(i) += mv.y;
                        }
                    }

                    _ => {
                        *s_transit.get_mut(i) = TRANSIT_IDLE;
                    }
                }
            } // unsafe
        }); // par_iter
    }
}
