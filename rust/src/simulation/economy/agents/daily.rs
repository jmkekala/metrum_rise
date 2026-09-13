// SPDX-License-Identifier: GPL-2.0-only

//! Low-frequency daily agent state updates.

use super::TRANSIT_IN_BUILDING;
use super::data::AgentSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use rayon::prelude::*;

impl AgentSystem {
    /// Update per-day agent state: home/work bonuses and pollution penalties.
    pub fn daily_update(
        &mut self,
        pollution: &PollutionSystem,
        config: &crate::simulation::core::config::WorldConfig,
    ) {
        let agents = &mut self.agents;
        (
            &mut agents.happiness,
            &mut agents.money,
            &agents.pos_x,
            &agents.pos_y,
            &agents.transit,
            &agents.activity,
        )
            .into_par_iter()
            // Daily work per agent is small; keep contiguous runs large enough to amortize dispatch.
            .with_min_len(8_192)
            .for_each(|(happiness, money, &x, &z, &transit, &activity)| {
                if transit == TRANSIT_IN_BUILDING && activity == 0 {
                    *happiness += 1.0;
                }

                let (gx_raw, gy_raw) = config.world_to_env_grid(x, z);
                let gx = gx_raw.round() as i32;
                let gy = gy_raw.round() as i32;
                if gx >= 0
                    && gy >= 0
                    && let Some(p) = pollution.grid.get(gx as usize, gy as usize)
                {
                    *happiness -= p * 0.1;
                }

                *happiness = happiness.clamp(0.0, 100.0);
                *money = money.max(0.0);
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::economy::agents::TRANSIT_NETWORK;

    #[test]
    fn daily_state_preserves_activity_pollution_bounds_and_clamping() {
        let config = WorldConfig {
            width_m: 80.0,
            height_m: 80.0,
            env_cell_m: 40.0,
            ..WorldConfig::default()
        };
        let mut pollution = PollutionSystem::new(&config);
        pollution
            .grid
            .data
            .copy_from_slice(&[10.0, 20.0, 30.0, 40.0]);
        let cases = [
            (-20.0, -20.0, TRANSIT_IN_BUILDING, 0, 50.0, 5.0, 50.0, 5.0),
            (20.0, -20.0, TRANSIT_IN_BUILDING, 1, 50.0, 5.0, 48.0, 5.0),
            (-20.0, 20.0, TRANSIT_NETWORK, 0, 50.0, 5.0, 47.0, 5.0),
            (20.0, 20.0, TRANSIT_IN_BUILDING, 0, 1.0, -4.0, 0.0, 0.0),
            (-60.0, -20.0, TRANSIT_IN_BUILDING, 0, 100.0, 5.0, 100.0, 5.0),
            (40.0, -20.0, TRANSIT_IN_BUILDING, 0, 50.0, 5.0, 51.0, 5.0),
            (-40.0, -20.0, TRANSIT_IN_BUILDING, 1, 10.0, 5.0, 10.0, 5.0),
            (20.0, 60.0, TRANSIT_IN_BUILDING, 0, -5.0, -4.0, 0.0, 0.0),
        ];
        for count in [cases.len(), 32_768] {
            let mut agents = AgentSystem::new();
            for i in 0..count {
                let (x, z, transit, activity, happiness, money, _, _) = cases[i % cases.len()];
                let id = agents.spawn_housed_agent(0, x, z);
                agents.transit[id] = transit;
                agents.activity[id] = activity;
                agents.happiness[id] = happiness;
                agents.money[id] = money;
            }
            agents.daily_update(&pollution, &config);
            for i in 0..count {
                let (_, _, _, _, _, _, happiness, money) = cases[i % cases.len()];
                assert_eq!(agents.happiness[i], happiness, "agent {i}");
                assert_eq!(agents.money[i], money, "agent {i}");
            }
        }
    }

    #[test]
    #[ignore = "manual matched release timing of daily agent state updates"]
    fn benchmark_daily_agent_update() {
        use std::{hint::black_box, time::Instant};
        let config = WorldConfig::default();
        let mut pollution = PollutionSystem::new(&config);
        pollution.grid.data.fill(3.0);
        for count in [1_000, 10_000, 100_000, 1_000_000] {
            let mut agents = AgentSystem::new();
            // Isolated SoA setup: no world placement, pathing, or household admission.
            for i in 0..count {
                let x = (i % 200) as f32 * 40.0 - 4_000.0;
                let z = (i / 200 % 200) as f32 * 40.0 - 4_000.0;
                let id = agents.spawn_housed_agent(0, x, z);
                agents.activity[id] = (i % 2) as u8;
            }
            for _ in 0..3 {
                agents.daily_update(&pollution, &config);
            }
            let mut samples = [0.0; 21];
            for sample in &mut samples {
                agents.happiness.fill(50.0);
                let start = Instant::now();
                for _ in 0..8 {
                    agents.daily_update(black_box(&pollution), black_box(&config));
                }
                *sample = start.elapsed().as_secs_f64() * 1_000.0 / 8.0;
            }
            let checksum = agents.happiness.iter().fold(0_u64, |hash, value| {
                hash.wrapping_mul(31)
                    .wrapping_add(u64::from(value.to_bits()))
            });
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "daily_agent_update agents={count} median_ms={:.6} checksum={checksum}",
                samples[10]
            );
        }
    }
}
