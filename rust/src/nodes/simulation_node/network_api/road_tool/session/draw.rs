// SPDX-License-Identifier: GPL-3.0-or-later

//! The road tool's drawing on the bridge (`ROAD-112` round R2-3): `road_tool_gizmos.gd` makes the
//! palette's materials once and attaches them with its world's scenario; from then on the tick
//! draws the gizmos and the batch ghost itself (`nodes::sim::road_tool::draw`), the hue is drawn
//! at the end of the process step that showed the frame's preview (`road_tool_draw_queued_hue`,
//! round HF2's pairing, queued by the tick since round R2-4b), and an accepted commit's hue is
//! held with its road chunks until `RoadDrawHolds` releases them (`session/overlay.rs`).

use super::tick::{TICK_GIZMOS, TICK_HUE};
use super::*;
use crate::nodes::sim::road_tool::draw::{DrawMaterials, OverlayState};
use godot::classes::{Material, ShaderMaterial};

#[godot_api(secondary)]
impl SimulationNode {
    /// Attaches the tool's drawing (once, from `road_tool_gizmos.gd`): meshes and instances in
    /// `scenario`, drawn with `line` (every line look, no depth test; its `dash_focus` follows
    /// the cursor), `depth_line` (the same looks depth-tested: the hue's tunnel outlines),
    /// `handle`, `hue` (the fill by state: valid, checking, refused) and `ghost_fills` (the batch
    /// ghost's fill: accepted, refused). Replaces an earlier attachment (whose objects it frees);
    /// the next tick draws everything again. False (nothing attached) when an array is short.
    #[func]
    pub fn road_tool_attach_draw(
        &mut self,
        scenario: Rid,
        line: Gd<ShaderMaterial>,
        depth_line: Gd<ShaderMaterial>,
        handle: Gd<Material>,
        hue: VarArray,
        ghost_fills: VarArray,
    ) -> bool {
        let materials = |array: &VarArray| -> Option<Vec<Gd<Material>>> {
            array.iter_shared().map(|item| item.try_to::<Gd<Material>>().ok()).collect()
        };
        let (Some(hue), Some(ghost_fills)) = (materials(&hue), materials(&ghost_fills)) else {
            return false;
        };
        if hue.len() != 3 || ghost_fills.len() != 2 {
            return false;
        }
        let handle_of = |material: &Gd<Material>| material.get_rid().to_u64();
        let handles = DrawMaterials {
            line: line.get_rid().to_u64(),
            depth_line: depth_line.get_rid().to_u64(),
            handle: handle_of(&handle),
            hue: [handle_of(&hue[0]), handle_of(&hue[1]), handle_of(&hue[2])],
            ghost_fill: [handle_of(&ghost_fills[0]), handle_of(&ghost_fills[1])],
        };
        let mut kept = vec![line.clone().upcast::<Material>(), depth_line.upcast(), handle];
        kept.extend(hue);
        kept.extend(ghost_fills);
        let bridge = &mut self.road_tool_session;
        // The earlier attachment's objects go first.
        bridge.draw = None;
        bridge.draw = Some(ToolDraw::new(GodotBackend::new(scenario, line, kept), handles));
        self.road_tool_forget_drawn(TICK_GIZMOS | TICK_HUE);
        true
    }

    /// Frees the tool's drawing (`road_tool_gizmos.gd` leaving the tree).
    #[func]
    pub fn road_tool_detach_draw(&mut self) {
        self.road_tool_session.draw = None;
        self.road_tool_forget_drawn(TICK_GIZMOS | TICK_HUE);
    }

    /// Draws the footprint hue on the tick's hue flag, after the frame showed its preview (round
    /// R2-2; `ROAD-123` round HF2: the frame's one wait for the build, which overlaps the
    /// preview's display): joins the build and draws it in place while its padded layout holds.
    /// Returns its state (-1 hidden, 0 valid, 1 checking, 2 refused). The join's and the draw's
    /// times go into the tick's sample (`road_tool_timings`' `hue_ms`, `hue_draw_ms`).
    /// `O(fill + outline points)`; allocates nothing once the buffers have grown, unless the
    /// layout changed.
    #[func]
    pub fn road_tool_draw_hue(&mut self) -> i64 {
        let start = Instant::now();
        let bridge = &mut self.road_tool_session;
        bridge.hue.join();
        let joined = Self::road_tool_tick_elapsed(start);
        let drawing = Instant::now();
        let hue = bridge.hue.hue();
        if let Some(draw) = &mut bridge.draw {
            draw.draw_hue(hue);
        }
        let state = hue.state().map_or(-1, |state| state as i64);
        let drawn = Self::road_tool_tick_elapsed(drawing);
        self.road_tool_tick_add(0.0, joined, drawn);
        state
    }

    /// The hue queued by the tick (`road_tool_queue_hue`, a deferred call at the end of the
    /// process step): joined and drawn as `road_tool_draw_hue`, for the preview the tick handed
    /// out last (`road_tool_hue_sequence`). Nothing when the drawings were dropped since.
    #[func]
    pub fn road_tool_draw_queued_hue(&mut self) {
        let tick = &mut self.road_tool_session.tick;
        if !tick.hue_queued {
            return;
        }
        tick.hue_queued = false;
        tick.hue_sequence = tick.sequence;
        self.road_tool_draw_hue();
    }

    /// The preview number of the hue drawn last (`road_tool_hue_times`' pose; the preview
    /// metrics), 0 before any.
    #[func]
    pub fn road_tool_hue_sequence(&self) -> i64 {
        self.road_tool_session.tick.hue_sequence
    }

    /// Draws (`show`) or hides the last tier 1's batch ghost (tests and captures; the tick draws
    /// its own poses' ghosts): `preview_road_draft`'s ghost after that call.
    #[func]
    pub fn road_tool_draw_batch_ghost(&mut self, show: bool) {
        let bridge = &mut self.road_tool_session;
        if let Some(draw) = &mut bridge.draw {
            if show {
                draw.draw_ghost(&bridge.ghost);
            } else {
                draw.clear_ghost();
            }
        }
    }

    /// What the tool's drawing shows (tests and probes): per overlay (`gizmos`, `hue`,
    /// `held_hue`, `ghost`, and `marks`: the preview's join marks, `ROAD-118` round JS5b) its
    /// mesh RID (`<name>_mesh`), whether it is shown (`<name>_visible`) and its surfaces
    /// (`<name>_surfaces`); `{}` while no drawing is attached.
    #[func]
    pub fn road_tool_drawn(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let Some(draw) = &self.road_tool_session.draw else {
            return dict;
        };
        let names = ["gizmos", "hue", "held_hue", "ghost", "marks"];
        for (name, state) in names.into_iter().zip(draw.state()) {
            let OverlayState {
                mesh,
                visible,
                surfaces,
            } = state;
            dict.set(format!("{name}_mesh").as_str(), Rid::new(mesh));
            dict.set(format!("{name}_visible").as_str(), visible);
            dict.set(format!("{name}_surfaces").as_str(), i64::from(surfaces));
        }
        dict
    }
}

impl SimulationNode {
    // An accepted commit's hue and road chunks stay drawn until the chunks of `generation`
    // install (`road_tool_hold_chunks`, `RoadDrawHolds`' rule); the next tick hands the hue over
    // again.
    pub(super) fn road_tool_hold_hue(&mut self, generation: i64) {
        if let Some(draw) = &mut self.road_tool_session.draw {
            draw.hold_hue();
        }
        self.road_tool_forget_drawn(TICK_HUE);
        self.road_tool_hold_chunks(generation);
    }

    // The tick's hue flag: the hue is joined and drawn at the end of the process step
    // (`road_tool_draw_queued_hue`, a deferred call: still before this frame draws, so the hue
    // and its preview show together, round HF2's pairing), once however many ticks raise it;
    // the build runs off the frame, so the frame's other nodes run beside it before the join
    // (round R2-4a). The deferred call is made once and reused: `O(1)`, no allocation.
    pub(super) fn road_tool_queue_hue(&mut self) {
        if self.road_tool_session.tick.hue_queued {
            return;
        }
        self.road_tool_session.tick.hue_queued = true;
        if self.road_tool_session.tick.hue_call.is_none() {
            let call = Callable::from_object_method(&self.to_gd(), "road_tool_draw_queued_hue");
            self.road_tool_session.tick.hue_call = Some(call);
        }
        if let Some(call) = &self.road_tool_session.tick.hue_call {
            call.call_deferred(&[]);
        }
    }
}
