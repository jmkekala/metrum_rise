//! Low-frequency daily agent state updates.

use super::TRANSIT_IN_BUILDING;
use super::data::AgentSystem;
use crate::simulation::grid::pollution::PollutionSystem;

impl AgentSystem {
    /// Update per-day agent state: home/work bonuses and pollution penalties.
    pub fn daily_update(
        &mut self,
        pollution: &PollutionSystem,
        config: &crate::simulation::core::config::WorldConfig,
    ) {
        let w = pollution.grid.width as f32;
        let h = pollution.grid.height as f32;

        for i in 0..self.agents.len() {
            if self.agents.transit[i] == TRANSIT_IN_BUILDING && self.agents.activity[i] == 0 {
                self.agents.happiness[i] += 1.0;
            }

            let (gx_raw, gy_raw) = config.world_to_env_grid(
                self.agents.pos_x[i],
                self.agents.pos_y[i],
                w as usize,
                h as usize,
            );
            let gx = gx_raw.round() as i32;
            let gy = gy_raw.round() as i32;
            if gx >= 0 && gx < w as i32 && gy >= 0 && gy < h as i32 {
                if let Some(p) = pollution.grid.get(gx as usize, gy as usize) {
                    self.agents.happiness[i] -= p * 0.1;
                }
            }

            self.agents.happiness[i] = self.agents.happiness[i].clamp(0.0, 100.0);
            self.agents.money[i] = self.agents.money[i].max(0.0);
        }
    }
}
