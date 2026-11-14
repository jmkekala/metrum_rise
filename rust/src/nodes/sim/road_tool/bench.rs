// SPDX-License-Identifier: GPL-2.0-only

//! The road tool session's benchmark fixture (`ROAD-112` round B), public so the Criterion bench
//! (`benches/road_tool_session_benchmark.rs`) can drive it: a fixed neighbourhood around the
//! origin and a background of block streets far east of it, the road tool's published view of
//! that city, and a scripted pointer trail of camera rays over the neighbourhood.
//!
//! The neighbourhood: an 800 m street grid of 100 m blocks centred on the origin (a junction at
//! the origin), with a dead-end spur and a free diagonal street inside two blocks, so the trail
//! meets nodes, edges, dead ends and open ground in a neighbourhood as dense as a city's (the
//! commit locality's neighbourhood is an 800 m street grid too). A tiny neighbourhood makes the
//! empty-city tier unrepresentative: the roadbed's region gathers walk the owner map instead of
//! their query cells while a city has fewer pieces than the query covers cells. The background:
//! `background / 10` straight 480 m two-lane streets from x = 800 m, nine to a row, rows 24 m
//! apart from z = -5,000 m (3,000 streets at 30k, more edges than the 30k populated city's
//! grid), beyond every fixed radius the trail's queries use. Built with the bulk road path,
//! outside timing. The scripted tests use the smaller [`NEIGHBOURHOOD`] instead.

use super::modes::DrawMode;
use super::pick::{CursorQuery, road_tool_cursor};
use super::session::{
    BORDER_SNAP_M, Pointer, PointerButton, RoadToolEvent, RoadToolOption, RoadToolSession,
    SNAP_RELEASE_M,
};
use crate::nodes::sim::core::{CityTreasury, RoadToolQuerySnapshot, SimCore};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::interaction::NetworkSnapTarget;
use godot::prelude::Vector3;

const WORLD_M: f32 = 12_288.0;
const TRAIL_MOVES: usize = 512;
// The stroke's start: the junction at the origin.
const START: Vector3 = Vector3::new(0.0, 0.0, 0.0);
// Camera height and its offset south of the aim point.
const CAMERA_HEIGHT_M: f32 = 400.0;
const CAMERA_BACK_M: f32 = 250.0;

/// The fixture: the view, a session with a stroke started at the junction, the control cursor's
/// retained snap and the pointer trail.
pub struct RoadToolBench {
    view: RoadToolQuerySnapshot,
    session: RoadToolSession,
    previous: Option<(NetworkSnapTarget, u64)>,
    rays: Vec<(Vector3, Vector3)>,
    edges: usize,
}

impl RoadToolBench {
    /// Builds the neighbourhood and `background` (in the locality tiers' units: one street per
    /// ten) and starts a straight stroke at the junction with every snap on (network, guide
    /// lines, zoning grid, a 15 degree angle lock and a 4 m length step), the worst case per move.
    pub fn build(background: usize) -> Self {
        let view = neighbourhood_view(true, background, WORLD_M);
        let edges = view.region_graph.edge_count();
        let mut bench = Self {
            view,
            session: RoadToolSession::new(),
            previous: None,
            rays: trail(),
            edges,
        };
        for option in [
            RoadToolOption::ZoningGrid(true),
            RoadToolOption::AngleStepDeg(Some(15.0)),
            RoadToolOption::LengthStepM(Some(4.0)),
        ] {
            bench.session.set_option(option);
        }
        bench.start_stroke(DrawMode::Straight);
        bench
    }

    /// Edges in the view's graph.
    pub fn edges(&self) -> usize {
        self.edges
    }

    /// Moves in the pointer trail.
    pub fn moves(&self) -> usize {
        self.rays.len()
    }

    /// Restarts the session's stroke in `mode` at the junction, so each move shapes that mode's
    /// draft: the simple curve gets its control point 150 m east of the start (a fillet arc), the
    /// S-curve its two aim points (a biarc), freeform two nodes (the move is the live third).
    pub fn restart(&mut self, mode: DrawMode) {
        self.start_stroke(mode);
        let placed: &[Vector3] = match mode {
            DrawMode::Straight => &[],
            DrawMode::SimpleCurve => &[Vector3::new(150.0, 0.0, 30.0)],
            DrawMode::ComplexCurve => &[
                Vector3::new(60.0, 0.0, 20.0),
                Vector3::new(120.0, 0.0, 60.0),
            ],
            DrawMode::Freeform => &[
                Vector3::new(60.0, 0.0, 40.0),
                Vector3::new(120.0, 0.0, -30.0),
            ],
        };
        for point in placed {
            self.place(*point);
        }
    }

    /// Restarts an S-curve at the junction before its start aim (`with_start_aim` false) or with
    /// only the start aim placed, so each move previews its first or second step (round SC:
    /// Straight's line from the start, then the simple curve's fillet through the start aim).
    pub fn restart_scurve_step(&mut self, with_start_aim: bool) {
        self.start_stroke(DrawMode::ComplexCurve);
        if with_start_aim {
            self.place(Vector3::new(60.0, 0.0, 20.0));
        }
    }

    /// Restarts a freeform draft of three nodes in its edit phase with its middle node grabbed,
    /// so each move drags that node (round N's edit phase).
    pub fn restart_freeform_drag(&mut self) {
        use super::session::RoadToolAction;
        self.restart(DrawMode::Freeform);
        self.place(Vector3::new(180.0, 0.0, 50.0));
        self.event(RoadToolEvent::Action(RoadToolAction::Confirm));
        // The middle node where the snaps put it (the frame lists the nodes' handles first).
        let node = self.session.frame().gizmos.handles[1];
        self.event(RoadToolEvent::Pointer {
            pointer: Pointer::Scripted(node),
            shift: false,
            ctrl: false,
        });
        self.event(RoadToolEvent::Press(PointerButton::Left));
        assert!(
            self.session.frame().commit.is_none(),
            "the press grabs the node"
        );
    }

    /// Restarts a straight stroke at the junction with straight grade on and the live end's
    /// handle raised 5 m (PageUp ten times), so each move builds a graded stroke (round I).
    pub fn restart_raised(&mut self) {
        use super::session::{RoadToolAction, RoadToolOption};
        self.restart(DrawMode::Straight);
        self.session.set_option(RoadToolOption::StraightGrade(true));
        for _ in 0..10 {
            self.event(RoadToolEvent::Action(RoadToolAction::RaiseHandle));
        }
    }

    /// Restarts a straight stroke from a block's middle with its start's height grip grabbed,
    /// so each move drags the grip (round I: the ray projected onto its vertical line).
    pub fn restart_height_drag(&mut self) {
        use super::session::RoadToolAction;
        self.event(RoadToolEvent::Action(RoadToolAction::Cancel));
        self.event(RoadToolEvent::Action(RoadToolAction::ModeStraight));
        self.place(Vector3::new(50.0, 0.0, 50.0));
        let grip = self.session.frame().height_handles[0]
            .expect("a free start's handle")
            .grip;
        self.event(RoadToolEvent::Pointer {
            pointer: Pointer::Scripted(grip),
            shift: false,
            ctrl: false,
        });
        self.event(RoadToolEvent::Press(PointerButton::Left));
        assert_eq!(self.session.placed_count(), 1, "the press grabs the grip");
    }

    // A pointer at a world point, then a left press there.
    fn place(&mut self, point: Vector3) {
        self.event(RoadToolEvent::Pointer {
            pointer: Pointer::World(point),
            shift: false,
            ctrl: false,
        });
        self.event(RoadToolEvent::Press(PointerButton::Left));
    }

    /// One session pointer move along the trail (pick, snap, shape, measure). Returns the
    /// cursor's x for `black_box`.
    pub fn session_move(&mut self, index: usize) -> f32 {
        let (origin, dir) = self.rays[index % self.rays.len()];
        self.event(RoadToolEvent::Pointer {
            pointer: Pointer::Ray { origin, dir },
            shift: false,
            ctrl: false,
        });
        self.session.frame().cursor.position.x
    }

    /// One control move: today's `get_road_tool_cursor_pos` logic (copied in `pick.rs`) for the
    /// same ray, placing the end of a straight stroke from the junction with the 45 degree
    /// magnet referenced east, the retained snap carried. Returns the cursor's x.
    pub fn cursor_move(&mut self, index: usize) -> f32 {
        let (ray_origin, ray_dir) = self.rays[index % self.rays.len()];
        let result = road_tool_cursor(
            &self.view,
            &CursorQuery {
                ray_origin,
                ray_dir,
                altitude_offset_m: 0.0,
                active: true,
                current_state: 2,
                start_pos: START,
                control_pos: START,
                zoning_snap_enabled: false,
                fwd_lanes: 1,
                bkw_lanes: 1,
                border_snap_dist_m: BORDER_SNAP_M,
                previous_snap: self.previous,
                sticky_release_dist_m: SNAP_RELEASE_M,
                angle_snap: Vector3::new(1.0, 0.0, 0.0),
            },
        );
        self.previous = result.and_then(|r| r.target.map(|t| (t, r.surface_generation)));
        result.map_or(0.0, |r| r.position.x)
    }

    /// A digest of the frame's bits (cursor, snap kind, stroke points, HUD), FNV-1a, for the
    /// locality products and determinism.
    pub fn frame_digest(&self) -> u64 {
        let frame = self.session.frame();
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        let mut eat = |bits: u64| {
            for byte in bits.to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
            }
        };
        let cursor = frame.cursor.position;
        for value in [cursor.x, cursor.y, cursor.z] {
            eat(u64::from(value.to_bits()));
        }
        eat(frame.cursor.snap as u64);
        for point in &frame.stroke.points {
            for value in [point.x, point.y, point.z] {
                eat(u64::from(value.to_bits()));
            }
        }
        let hud = frame.hud;
        for value in [
            hud.length_m,
            hud.bearing_deg,
            hud.build_cost,
            hud.upkeep_per_day,
        ] {
            eat(value.to_bits());
        }
        for value in [hud.start_angle_deg, hud.end_angle_deg, hud.radius_m] {
            eat(value.map_or(u64::MAX, f64::to_bits));
        }
        hash
    }

    fn start_stroke(&mut self, mode: DrawMode) {
        use super::session::RoadToolAction;
        let action = match mode {
            DrawMode::Straight => RoadToolAction::ModeStraight,
            DrawMode::SimpleCurve => RoadToolAction::ModeSimpleCurve,
            DrawMode::ComplexCurve => RoadToolAction::ModeComplexCurve,
            DrawMode::Freeform => RoadToolAction::ModeFreeform,
        };
        self.event(RoadToolEvent::Action(action));
        self.event(RoadToolEvent::Pointer {
            pointer: Pointer::World(START),
            shift: false,
            ctrl: false,
        });
        self.event(RoadToolEvent::Press(PointerButton::Left));
    }

    fn event(&mut self, event: RoadToolEvent) {
        self.session.handle_event(&self.view, event);
    }
}

/// The fixture's city as the road tool's published view: the neighbourhood (the bench's street
/// grid when `grid`, else the tests' [`NEIGHBOURHOOD`]) and `background` (one street per ten) in
/// a square world `world_m` wide (at least 11 km for a background).
pub(crate) fn neighbourhood_view(
    grid: bool,
    background: usize,
    world_m: f32,
) -> RoadToolQuerySnapshot {
    let mut core = SimCore {
        treasury: CityTreasury::new(1.0e12),
        benchmark_mode: true,
        ..SimCore::new(WorldConfig::new(world_m, world_m, 40.0, 10.0))
    };
    core.precompute_road_mesh_data();
    core.transit_network.bulk_load = true;
    let road = |core: &mut SimCore, a: (f32, f32), b: (f32, f32)| {
        let outcome = core.add_road_internal(
            vec![Vector3::new(a.0, 0.0, a.1), Vector3::new(b.0, 0.0, b.1)],
            1,
            1,
        );
        assert!(
            outcome.committed,
            "fixture road {a:?} -> {b:?}: {}",
            core.last_road_timing
        );
        core.transit_network
            .roadbed
            .compile_dirty(&core.region_graph);
    };
    if grid {
        // Block by block, so each street meets the junctions already there.
        for line in -4..=4 {
            for block in -4..4 {
                let (along, next) = (block as f32 * 100.0, (block + 1) as f32 * 100.0);
                let across = line as f32 * 100.0;
                road(&mut core, (across, along), (across, next));
                road(&mut core, (along, across), (next, across));
            }
        }
        road(&mut core, (-150.0, 200.0), (-150.0, 260.0));
        road(&mut core, (120.0, 120.0), (180.0, 180.0));
    } else {
        for (a, b) in NEIGHBOURHOOD {
            road(&mut core, a, b);
        }
    }
    for street in 0..background / 10 {
        let z = -5_000.0 + (street / 9) as f32 * 24.0;
        let west = 800.0 + (street % 9) as f32 * 504.0;
        road(&mut core, (west, z), (west + 480.0, z));
    }
    {
        let core = &mut core;
        core.transit_network
            .finalize_bulk_load(&mut core.region_graph, &mut core.allocator);
        core.transit_network
            .roadbed
            .compile_dirty(&core.region_graph);
    }
    // The view as the simulation publishes it to the road tool.
    core.publish_road_tool_view()
        .expect("a current roadbed")
        .view
}

/// The neighbourhood's roads, in build order (their edge ids follow it, split pieces after).
pub(crate) const NEIGHBOURHOOD: [((f32, f32), (f32, f32)); 7] = [
    ((-300.0, 0.0), (0.0, 0.0)),
    ((0.0, 0.0), (300.0, 0.0)),
    ((0.0, -300.0), (0.0, 0.0)),
    ((0.0, 0.0), (0.0, 300.0)),
    ((-300.0, -60.0), (300.0, -60.0)),
    ((-120.0, 120.0), (-40.0, 200.0)),
    ((60.0, 60.0), (220.0, 180.0)),
];

// A loop of camera rays aimed at points circling the origin at 40-260 m: over the grid's
// streets and junctions, the spur's dead end, the diagonal and the open block interiors.
fn trail() -> Vec<(Vector3, Vector3)> {
    (0..TRAIL_MOVES)
        .map(|index| {
            let turn = index as f32 / TRAIL_MOVES as f32 * std::f32::consts::TAU;
            let radius = 150.0 + 110.0 * (3.0 * turn).sin();
            let aim = Vector3::new(radius * turn.cos(), 0.0, radius * turn.sin());
            let origin = Vector3::new(aim.x, CAMERA_HEIGHT_M, aim.z + CAMERA_BACK_M);
            (origin, (aim - origin).normalized())
        })
        .collect()
}
