// SPDX-License-Identifier: GPL-2.0-only

//! The road tool's drawing on the bridge (`ROAD-112` round R2-3): `road_tool_gizmos.gd` makes the
//! palette's materials once and attaches them with its world's scenario; from then on the tick
//! draws the gizmos and the batch ghost itself (`nodes::sim::road_tool::draw`), the script asks
//! for the hue after it showed the frame's preview (`road_tool_draw_hue`, round HF2's pairing),
//! and an accepted commit's hue is held in Rust until the script releases the commit's preview.

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

    /// The held commit's hue and overlay chunks go (`road_tool.gd` released the commit's
    /// preview: its chunks installed, or the preview was reset).
    #[func]
    pub fn road_tool_release_held(&mut self) {
        if let Some(draw) = &mut self.road_tool_session.draw {
            draw.release_held_hue();
        }
        self.road_tool_release_held_overlay();
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
    /// `held_hue`, `ghost`) its mesh RID (`<name>_mesh`), whether it is shown (`<name>_visible`)
    /// and its surfaces (`<name>_surfaces`); `{}` while no drawing is attached.
    #[func]
    pub fn road_tool_drawn(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let Some(draw) = &self.road_tool_session.draw else {
            return dict;
        };
        let names = ["gizmos", "hue", "held_hue", "ghost"];
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
    // An accepted commit's hue and overlay chunks stay drawn until its preview's hold releases
    // (`road_tool_release_held`); the next tick hands the hue over again.
    pub(super) fn road_tool_hold_hue(&mut self) {
        if let Some(draw) = &mut self.road_tool_session.draw {
            draw.hold_hue();
        }
        self.road_tool_hold_overlay();
        self.road_tool_forget_drawn(TICK_HUE);
    }
}
