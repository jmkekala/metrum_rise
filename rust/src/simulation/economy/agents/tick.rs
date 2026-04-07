//! Main simulation loop for agents: transit state machine and movement.

use super::data::AgentSystem;
use super::{
    MODE_CAR, MODE_WALK, TRANSIT_ARRIVING, TRANSIT_DEPARTING, TRANSIT_IDLE, TRANSIT_IMMIGRATING,
    TRANSIT_INTERSECTION, TRANSIT_ON_ROAD,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::network::TransitNetwork;
use crate::simulation::pathing::flow_field::FlowField;
use godot::prelude::*;
use rand::Rng;
use rayon::prelude::*;
use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};

// ---------------------------------------------------------------------------
// IDM (Intelligent Driver Model) constants — defined in config.rs.
// ---------------------------------------------------------------------------

use crate::config::{CAR_LENGTH, IDM_A_MAX, IDM_S_MIN, IDM_T_HEAD};

/// Returns the bumper-to-bumper gap to the nearest vehicle ahead in a pre-sorted
/// per-lane bucket. `bucket` must be sorted ascending by distance.
fn idm_gap_bucket(bucket: &[(f32, usize)], my_dist: f32) -> f32 {
    let ahead = bucket.partition_point(|e| e.0 <= my_dist + 0.05);
    if ahead < bucket.len() {
        (bucket[ahead].0 - my_dist - CAR_LENGTH).max(0.1)
    } else {
        f32::MAX
    }
}

/// Dispatches `f` over `0..n` sequentially when `n < PAR_THRESHOLD`, otherwise in
/// parallel via Rayon.  Below the threshold Rayon's worker threads would spin-wait
/// for ~1 ms after each call looking for more work; at 60 Hz with 3 parallel
/// sections per tick that idle spin accounts for ~1–2 extra CPU cores even when
/// the city has only a few hundred agents.
const PAR_THRESHOLD: usize = 500;

fn dispatch_agents<F: Fn(usize) + Send + Sync>(n: usize, f: F) {
    if n >= PAR_THRESHOLD {
        (0..n).into_par_iter().for_each(f);
    } else {
        (0..n).for_each(f);
    }
}

/// Returns the new speed for one IDM time step. Uses the simplified IDM without the
/// approach-speed interaction term (`v·Δv / 2√(a_max·b)`) — the full term can be
/// added once per-agent `v_lead` tracking is in place.
fn idm_new_speed(v: f32, v_max: f32, gap: f32, dt: f32) -> f32 {
    let free = (v / v_max.max(0.1)).powi(4);
    let acc  = if gap < f32::MAX / 2.0 {
        let s_star = IDM_S_MIN + v * IDM_T_HEAD;
        IDM_A_MAX * (1.0 - free - (s_star / gap).powi(2))
    } else {
        IDM_A_MAX * (1.0 - free)
    };
    (v + acc * dt).clamp(0.0, v_max)
}

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

/// Disjoint SoA slices used by `process_agent_movement` for parallel data access.
pub(crate) struct MovementSlices {
    home: RawSlice<usize>,
    work: RawSlice<usize>,
    pos_x: RawSlice<f32>,
    pos_y: RawSlice<f32>,
    visible: RawSlice<bool>,
    activity: RawSlice<u8>,
    transit: RawSlice<u8>,
    happiness: RawSlice<f32>,
    jstart: RawSlice<f32>,
    cur_b: RawSlice<usize>,
    tgt_b: RawSlice<usize>,
    planned_tgt_b: RawSlice<usize>,
    cur_n: RawSlice<u32>,
    tgt_n: RawSlice<u32>,
    cur_e: RawSlice<usize>,
    lane_id: RawSlice<usize>,
    lane_d: RawSlice<f32>,
    tmode: RawSlice<u8>,
    planned_activity: RawSlice<u8>,
    path: RawSlice<Vec<u32>>,
    path_idx: RawSlice<usize>,
    has_car: RawSlice<bool>,
    speed: RawSlice<f32>,
    walk_phase: RawSlice<f32>,
}

impl AgentSystem {
    /// Advances the agent simulation by `delta` seconds.
    pub fn tick(
        &mut self,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &mut RegionGraph,
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
        let s_plan_b   = RawSlice::new(&mut self.agents.planned_target_building);
        let s_transit  = RawSlice::new(&mut self.agents.transit);
        let s_visible  = RawSlice::new(&mut self.agents.is_visible);

        dispatch_agents(n, |i| unsafe {
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
            let planned = *s_plan_b.get(i);
            if planned != usize::MAX && planned >= bldg_count {
                *s_plan_b.get_mut(i) = usize::MAX;
            }
        });

        // -----------------------------------------------------------------------
        // 2. Lane bucket fill — sequential O(A).
        // -----------------------------------------------------------------------
        let lane_count = transit_network.lane_system.lanes.len();
        if self.lane_buckets.len() < lane_count {
            self.lane_buckets.resize_with(lane_count, Vec::new);
            self.lane_is_dirty.resize(lane_count, false);
        }
        for &lid in &self.dirty_lanes {
            self.lane_buckets[lid].clear();
            self.lane_is_dirty[lid] = false;
        }
        self.dirty_lanes.clear();
        for i in 0..n {
            if self.agents.transit[i] == TRANSIT_ON_ROAD {
                let lid = self.agents.current_lane_id[i];
                if lid != usize::MAX && lid < lane_count {
                    if !self.lane_is_dirty[lid] {
                        self.lane_is_dirty[lid] = true;
                        self.dirty_lanes.push(lid);
                    }
                    self.lane_buckets[lid].push((self.agents.lane_distance[i], i));
                }
            }
        }
        // Parallel sort over dirty lanes. Each lid's Vec is disjoint → safe.
        // Safety: dirty_lanes has no duplicates, so each iteration accesses a
        // distinct element of lane_buckets — no data races.
        {
            let buckets_raw = RawSlice::new(&mut self.lane_buckets);
            if self.dirty_lanes.len() >= PAR_THRESHOLD {
                self.dirty_lanes.par_iter().for_each(|&lid| {
                    let bucket = unsafe { buckets_raw.get_mut(lid) };
                    bucket.sort_unstable_by(|a, b| {
                        a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
                    });
                });
            } else {
                for &lid in &self.dirty_lanes {
                    self.lane_buckets[lid].sort_unstable_by(|a, b| {
                        a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
            }
        }

        // Build junction gate snapshot.
        self.build_conn_occupied_snapshot(lane_count);

        // -----------------------------------------------------------------------
        // 3. IDM speed update — parallel.
        // -----------------------------------------------------------------------
        self.new_speed.resize(n, 0.0_f32);
        {
            let s_transit_idm = RawSlice::new(&mut self.agents.transit);
            let s_tmode_idm   = RawSlice::new(&mut self.agents.transit_mode);
            let s_lane_idm    = RawSlice::new(&mut self.agents.current_lane_id);
            let s_lane_d_idm  = RawSlice::new(&mut self.agents.lane_distance);
            let s_cur_e_idm   = RawSlice::new(&mut self.agents.current_edge);
            let s_speed_idm   = RawSlice::new(&mut self.agents.speed);
            let new_spd_raw   = RawSlice { ptr: self.new_speed.as_mut_ptr(), len: n };
            let buckets: &Vec<Vec<(f32, usize)>> = &self.lane_buckets;

            dispatch_agents(n, |i| {
                unsafe {
                    let cur_spd = *s_speed_idm.get(i);
                    let transit = *s_transit_idm.get(i);
                    let tmode   = *s_tmode_idm.get(i);

                    if transit != TRANSIT_ON_ROAD || tmode != MODE_CAR {
                        *new_spd_raw.get_mut(i) = cur_spd;
                        return;
                    }

                    let lid  = *s_lane_idm.get(i);
                    let my_d = *s_lane_d_idm.get(i);
                    let eid  = *s_cur_e_idm.get(i);

                    let v_max = if eid != usize::MAX && eid < graph.edge_count() {
                        graph.edge(eid).speed_limit
                    } else {
                        20.0_f32
                    };

                    let gap = if lid < buckets.len() {
                        idm_gap_bucket(&buckets[lid], my_d)
                    } else {
                        f32::MAX
                    };
                    *new_spd_raw.get_mut(i) = idm_new_speed(cur_spd, v_max, gap, delta);
                }
            });
        }
        for i in 0..n {
            self.agents.speed[i] = self.new_speed[i];
        }

        // -----------------------------------------------------------------------
        // 4. Main agent loop — parallel.
        // -----------------------------------------------------------------------
        let slices = MovementSlices {
            home: RawSlice::new(&mut self.agents.home_building),
            work: RawSlice::new(&mut self.agents.work_building),
            pos_x: RawSlice::new(&mut self.agents.pos_x),
            pos_y: RawSlice::new(&mut self.agents.pos_y),
            visible: RawSlice::new(&mut self.agents.is_visible),
            activity: RawSlice::new(&mut self.agents.activity),
            transit: RawSlice::new(&mut self.agents.transit),
            happiness: RawSlice::new(&mut self.agents.happiness),
            jstart: RawSlice::new(&mut self.agents.journey_start_time),
            cur_b: RawSlice::new(&mut self.agents.current_building),
            tgt_b: RawSlice::new(&mut self.agents.target_building),
            planned_tgt_b: RawSlice::new(&mut self.agents.planned_target_building),
            cur_n: RawSlice::new(&mut self.agents.current_node),
            tgt_n: RawSlice::new(&mut self.agents.target_node),
            cur_e: RawSlice::new(&mut self.agents.current_edge),
            lane_id: RawSlice::new(&mut self.agents.current_lane_id),
            lane_d: RawSlice::new(&mut self.agents.lane_distance),
            tmode: RawSlice::new(&mut self.agents.transit_mode),
            planned_activity: RawSlice::new(&mut self.agents.planned_activity),
            path: RawSlice::new(&mut self.agents.current_path),
            path_idx: RawSlice::new(&mut self.agents.current_path_index),
            has_car: RawSlice::new(&mut self.agents.has_car),
            speed: RawSlice::new(&mut self.agents.speed),
            walk_phase: RawSlice::new(&mut self.agents.walk_phase),
        };

        let conn_occupied = &self.conn_occupied;
        let sim_time = self.sim_time;

        dispatch_agents(n, |i| unsafe {
            Self::process_agent_movement(
                i,
                delta,
                sim_time,
                allocator,
                transit_network,
                graph,
                &self.pathfind_count,
                conn_occupied,
                &slices,
            );
        });

        // -----------------------------------------------------------------------
        // 5. Post-movement overlap correction + congestion accumulation — sequential O(A).
        //    Merged into one pass to avoid a second O(A) scan in write_congestion.
        // -----------------------------------------------------------------------
        {
            for &lid in &self.dirty_lanes {
                self.lane_buckets[lid].clear();
                self.lane_is_dirty[lid] = false;
            }
            self.dirty_lanes.clear();

            let edge_count = graph.edge_count();
            self.edge_speed_sum.clear();
            self.edge_speed_sum.resize(edge_count, 0.0_f32);
            self.edge_agent_cnt.clear();
            self.edge_agent_cnt.resize(edge_count, 0_u32);

            for i in 0..n {
                if self.agents.transit[i] == TRANSIT_ON_ROAD {
                    let lid = self.agents.current_lane_id[i];
                    if lid != usize::MAX && lid < lane_count {
                        if !self.lane_is_dirty[lid] {
                            self.lane_is_dirty[lid] = true;
                            self.dirty_lanes.push(lid);
                        }
                        self.lane_buckets[lid].push((self.agents.lane_distance[i], i));
                    }
                    let eid = self.agents.current_edge[i];
                    if eid != usize::MAX && eid < edge_count {
                        self.edge_speed_sum[eid] += self.agents.speed[i];
                        self.edge_agent_cnt[eid] += 1;
                    }
                }
            }

            // Parallel sort + overlap correction.
            // Safety: dirty_lanes has no duplicates → each lid accesses a distinct Vec.
            let min_sep = CAR_LENGTH + IDM_S_MIN;
            {
                let buckets_raw = RawSlice::new(&mut self.lane_buckets);
                if self.dirty_lanes.len() >= PAR_THRESHOLD {
                    self.dirty_lanes.par_iter().for_each(|&lid| {
                        let bucket = unsafe { buckets_raw.get_mut(lid) };
                        bucket.sort_unstable_by(|a, b| {
                            a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
                        });
                    });
                } else {
                    for &lid in &self.dirty_lanes {
                        self.lane_buckets[lid].sort_unstable_by(|a, b| {
                            a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
                        });
                    }
                }
            }
            for &lid in &self.dirty_lanes {
                let bucket = &mut self.lane_buckets[lid];
                for j in (0..bucket.len().saturating_sub(1)).rev() {
                    let max_rear = (bucket[j + 1].0 - min_sep).max(0.0);
                    if bucket[j].0 > max_rear {
                        bucket[j].0 = max_rear;
                        self.agents.lane_distance[bucket[j].1] = max_rear;
                    }
                }
            }

            // 6. Commit congestion — O(E).
            for eid in 0..edge_count {
                if !graph.edge(eid).deleted && self.edge_agent_cnt[eid] > 0 {
                    let avg = self.edge_speed_sum[eid] / self.edge_agent_cnt[eid] as f32;
                    let limit = graph.edge(eid).speed_limit.max(1.0);
                    graph.set_edge_congestion(eid, (1.0 - avg / limit).max(0.0));
                }
            }
        }
    }

    /// Core agent movement logic (FSM and physics).
    /// Safety: Caller must ensure disjoint access to agent SoA via `MovementSlices`.
    #[inline(always)]
    pub(crate) unsafe fn process_agent_movement(
        i: usize,
        delta: f32,
        sim_time: f32,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        pathfind_count: &AtomicU32,
        conn_occupied: &Vec<bool>,
        slices: &MovementSlices,
    ) {
        let mut rng = rand::thread_rng();

        // Safety: index i is unique to this thread via par_iter.
        unsafe {
            let s_cur_n      = &slices.cur_n;
            let s_tgt_n      = &slices.tgt_n;
            let s_tmode      = &slices.tmode;
            let s_speed      = &slices.speed;
            let s_walk_phase = &slices.walk_phase;
            let s_transit    = &slices.transit;
            let s_activity   = &slices.activity;
            let s_work       = &slices.work;
            let s_home       = &slices.home;
            let s_cur_b      = &slices.cur_b;
            let s_tgt_b      = &slices.tgt_b;
            let s_plan_b     = &slices.planned_tgt_b;
            let s_has_car    = &slices.has_car;
            let s_jstart     = &slices.jstart;
            let s_path       = &slices.path;
            let s_path_idx   = &slices.path_idx;
            let s_lane_id    = &slices.lane_id;
            let s_lane_d     = &slices.lane_d;
            let s_visible    = &slices.visible;
            let s_pos_x      = &slices.pos_x;
            let s_pos_y      = &slices.pos_y;
            let s_cur_e      = &slices.cur_e;
            let s_happiness  = &slices.happiness;
            let s_plan_act   = &slices.planned_activity;

            *s_cur_n.get_mut(i) = graph.get_valid_node(*s_cur_n.get(i));
            *s_tgt_n.get_mut(i) = graph.get_valid_node(*s_tgt_n.get(i));

            // Update walk animation phase if not in a vehicle.
            if *s_tmode.get(i) != MODE_CAR {
                let spd = *s_speed.get(i);
                let phase = *s_walk_phase.get(i);
                // Cycle: about 1 time per meter traveled.
                *s_walk_phase.get_mut(i) = (phase + (spd.abs() * 0.8 * delta)) % 1.0;
            }

            match *s_transit.get(i) {
                TRANSIT_IDLE => {
                    let next_bldg = *s_plan_b.get(i);
                    let next_act = *s_plan_act.get(i);
                    let curr_bldg = *s_cur_b.get(i);
                    if next_bldg != usize::MAX
                        && next_bldg < allocator.buildings.len()
                        && curr_bldg != usize::MAX
                        && curr_bldg < allocator.buildings.len()
                    {
                        let origin_node =
                            crate::simulation::buildings::allocator::building_depart_node(
                                &allocator.buildings[curr_bldg],
                                graph,
                            );
                        let target_node =
                            crate::simulation::buildings::allocator::building_depart_node(
                                &allocator.buildings[next_bldg],
                                graph,
                            );
                        *s_cur_n.get_mut(i) = origin_node;
                        *s_tgt_n.get_mut(i) = target_node;

                        let has_car = *s_has_car.get(i);
                        let mode = if has_car { MODE_CAR } else { MODE_WALK };
                        let search_flags = if has_car {
                            TransitFlags::CAR
                        } else {
                            TransitFlags::FOOT
                        };

                        let target_zone = if next_act == 1 {
                            Some(allocator.buildings[next_bldg].zone_type)
                        } else {
                            None
                        };
                        let ff: Option<&FlowField> = target_zone.and_then(|z| {
                            if has_car {
                                transit_network.flow_fields.car(z)
                            } else {
                                transit_network.flow_fields.foot(z)
                            }
                        });

                        let path_opt: Option<Vec<u32>> = ff
                            .and_then(|f| f.build_path(origin_node, graph.node_count() + 1))
                            .filter(|p| crate::simulation::pathing::cch::CchGraph::path_has_valid_turns(p, graph))
                            .or_else(|| {
                                pathfind_count.fetch_add(1, Ordering::Relaxed);
                                transit_network
                                    .cch_graph
                                    .find_path(origin_node, target_node, usize::MAX, graph, search_flags)
                                    .map(|(_, _, p)| p)
                            });

                        if let Some(path) = path_opt {
                            let effective_tgt = if let Some(f) = ff {
                                let dest_node = path.last().copied().unwrap_or(target_node);
                                let nearest = f.nearest_building[origin_node as usize];
                                if nearest != usize::MAX
                                    && crate::simulation::buildings::allocator::building_depart_node(
                                        &allocator.buildings[nearest],
                                        graph,
                                    ) == dest_node
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
                        *s_plan_b.get_mut(i) = usize::MAX;
                        *s_plan_act.get_mut(i) = 0;
                    }
                }

                TRANSIT_DEPARTING => {
                    let b_id = *s_cur_b.get(i);
                    if b_id == usize::MAX || b_id >= allocator.buildings.len() {
                        *s_transit.get_mut(i) = TRANSIT_ON_ROAD;
                        return;
                    }
                    let frontage_node = crate::simulation::buildings::allocator::building_depart_node(&allocator.buildings[b_id], graph);
                    if frontage_node as usize >= graph.node_count() {
                        *s_transit.get_mut(i) = TRANSIT_IDLE;
                        *s_visible.get_mut(i) = false;
                        return;
                    }
                    let node_pos = graph.node(frontage_node).pos;
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
                        if *s_transit.get(i) == TRANSIT_INTERSECTION {
                            // Slow through intersections; still IDM-bounded.
                            (*s_speed.get(i) * 0.5).max(2.0)
                        } else {
                            *s_speed.get(i)
                        }
                    } else {
                        4.0 // pedestrians use a fixed speed; IDM is car-only
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

                            // If the agent has a known incoming edge at the current node,
                            // the flow field cannot account for turn restrictions — skip it
                            // and let CCH enforce the constraint via start_edge.
                            let incoming_edge = *s_cur_e.get(i);
                            let ff_blocked = incoming_edge != usize::MAX;

                            let path_opt: Option<Vec<u32>> = if ff_blocked { None } else {
                                ff.and_then(|f| f.build_path(cur_n, graph.node_count() + 1))
                                  .filter(|p| p.len() > 1)
                                  .filter(|p| crate::simulation::pathing::cch::CchGraph::path_has_valid_turns(p, graph))
                            }.or_else(|| {
                                pathfind_count.fetch_add(1, Ordering::Relaxed);
                                transit_network.cch_graph
                                    .find_path(cur_n, tgt_n, incoming_edge, graph, search_flags)
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
                                    let edge = graph.edge(best_e);
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
                                                // Seed speed from edge limit on first lane entry.
                                                if *s_speed.get(i) == 0.0 {
                                                    *s_speed.get_mut(i) = graph.edge(best_e).speed_limit;
                                                }
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
                                    let tgt_len = graph.edge(b.edge_idx).physical_length;
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
                                    graph.edge(lane.edge_id).end_node
                                } else {
                                    graph.edge(lane.edge_id).start_node
                                };

                                *s_path_idx.get_mut(i) += 1;
                                let path_idx = *s_path_idx.get(i);
                                let path_len = s_path.get(i).len();

                                if path_idx < path_len {
                                    let next_node = s_path.get(i)[path_idx];
                                    if let Some(best_e) = graph.get_edge_between_nodes(*s_cur_n.get(i), next_node) {
                                        let mut wait_for_gap = false;
                                        let cur_node_idx = *s_cur_n.get(i) as usize;
                                        let is_junction = graph.node_adjacency(cur_node_idx as u32).len() >= 3;
                                        VALID_CONNS.with(|v| {
                                            let mut valid_conns = v.borrow_mut();
                                            valid_conns.clear();
                                            let mut any_routing_valid = false;
                                            for &c_id in &lane.next_lanes {
                                                if c_id < transit_network.lane_system.lanes.len() {
                                                    let conn_lane = &transit_network.lane_system.lanes[c_id];
                                                    if !conn_lane.next_lanes.is_empty() {
                                                        let tgt_road_lane = conn_lane.next_lanes[0];
                                                        if tgt_road_lane < transit_network.lane_system.lanes.len()
                                                            && transit_network.lane_system.lanes[tgt_road_lane].edge_id == best_e
                                                        {
                                                            any_routing_valid = true;
                                                            let occupied = is_junction
                                                                && conn_occupied.get(c_id).copied().unwrap_or(false);
                                                            if !occupied {
                                                                valid_conns.push(c_id);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            if !valid_conns.is_empty() {
                                                *s_lane_id.get_mut(i) = valid_conns[rng.gen_range(0..valid_conns.len())];
                                                *s_lane_d.get_mut(i) = 0.0;
                                                *s_transit.get_mut(i) = TRANSIT_INTERSECTION;
                                                *s_cur_e.get_mut(i) = usize::MAX;
                                            } else if any_routing_valid {
                                                *s_path_idx.get_mut(i) -= 1;
                                                *s_lane_d.get_mut(i) = lane.length;
                                                wait_for_gap = true;
                                            } else {
                                                // No connection lane exists for this turn.
                                                // Clear the path so the agent re-pathfinds on
                                                // the next tick — the updated CCH will now route
                                                // around the restricted junction.
                                                s_path.get_mut(i).clear();
                                                *s_lane_id.get_mut(i) = usize::MAX;
                                            }
                                        });
                                        if wait_for_gap {
                                            break;
                                        }
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
                                        let frontage = crate::simulation::buildings::allocator::building_depart_node(&allocator.buildings[t_bldg], graph);
                                        if *s_cur_n.get(i) == frontage {
                                            *s_transit.get_mut(i) = TRANSIT_ARRIVING;
                                            *s_tmode.get_mut(i) = MODE_WALK;
                                        }
                                    }
                                    break;
                                }
                            } else {
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
                    if b_id == usize::MAX || b_id >= allocator.buildings.len() {
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
                        *s_pos_x.get_mut(i) = center_vec.x;
                        *s_pos_y.get_mut(i) = center_vec.y;
                        *s_cur_b.get_mut(i) = b_id;
                        *s_visible.get_mut(i) = false;
                        *s_transit.get_mut(i) = TRANSIT_IDLE;
                        let home = *s_home.get(i);
                        let work = *s_work.get(i);
                        if b_id == home { *s_activity.get_mut(i) = 0; }
                        else if b_id == work { *s_activity.get_mut(i) = 1; }
                        else { *s_activity.get_mut(i) = 2; }
                        *s_tmode.get_mut(i) = MODE_WALK;
                        *s_cur_e.get_mut(i) = usize::MAX;
                        *s_lane_id.get_mut(i) = usize::MAX;
                        *s_lane_d.get_mut(i) = 0.0;
                        s_path.get_mut(i).clear();
                        *s_path_idx.get_mut(i) = 0;

                        let commute_time = sim_time - *s_jstart.get(i);
                        *s_happiness.get_mut(i) = (*s_happiness.get(i) - commute_time / 60.0).clamp(0.0, 100.0);
                    } else if dist > 0.0001 {
                        let mv = dir_to_center.normalized() * step;
                        *s_pos_x.get_mut(i) += mv.x;
                        *s_pos_y.get_mut(i) += mv.y;
                    }
                }
                _ => { *s_transit.get_mut(i) = TRANSIT_IDLE; }
            }
        }
    }

    /// Build junction gate snapshot: mark every connection lane that already has an
    /// agent in TRANSIT_INTERSECTION.
    pub(crate) fn build_conn_occupied_snapshot(&mut self, lane_count: usize) {
        let n = self.agents.len();
        if self.conn_occupied.len() < lane_count {
            self.conn_occupied.resize(lane_count, false);
        }
        self.conn_occupied.fill(false);
        for i in 0..n {
            if self.agents.transit[i] == TRANSIT_INTERSECTION {
                let lid = self.agents.current_lane_id[i];
                if lid < self.conn_occupied.len() {
                    self.conn_occupied[lid] = true;
                }
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::dispatch_agents;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Verifies that `dispatch_agents` visits every index in `0..n` exactly once,
    /// both below the PAR_THRESHOLD (sequential path) and above it (parallel path).
    #[test]
    fn test_dispatch_agents_visits_each_index_once() {
        for n in [10_usize, 499, 500, 501, 600] {
            let counts: Vec<AtomicUsize> = (0..n).map(|_| AtomicUsize::new(0)).collect();
            dispatch_agents(n, |i| {
                counts[i].fetch_add(1, Ordering::Relaxed);
            });
            for (i, c) in counts.iter().enumerate() {
                assert_eq!(
                    c.load(Ordering::Relaxed),
                    1,
                    "n={n}: index {i} was visited {} time(s), expected 1",
                    c.load(Ordering::Relaxed)
                );
            }
        }
    }
}
