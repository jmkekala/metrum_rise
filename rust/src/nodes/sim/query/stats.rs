//! Global simulation statistics and demand counters.

use crate::nodes::sim::core::SimCore;
use godot::prelude::*;

impl SimCore {
    /// Returns the demographic statistics for the city.
    pub fn get_city_demographics_internal(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();

        // Calculate population
        let pop = self.agents.len();

        let mut employed = 0;
        let mut sum_happiness = 0.0;
        let mut sum_wealth = 0.0;

        if pop > 0 {
            for i in 0..pop {
                if self.agents.work_building[i] != usize::MAX {
                    employed += 1;
                }
                sum_happiness += self.agents.happiness[i];
                sum_wealth += self.agents.money[i];
            }
            let emp_rate = (employed as f32 / pop as f32) * 100.0;
            let avg_hap = sum_happiness / pop as f32;
            let avg_wealth = sum_wealth / pop as f32;

            dict.set("population", pop as i32);
            dict.set("employment_rate", emp_rate);
            dict.set("average_happiness", avg_hap);
            dict.set("average_wealth", avg_wealth);
        } else {
            dict.set("population", 0_i32);
            dict.set("employment_rate", 0.0_f32);
            dict.set("average_happiness", 100.0_f32);
            dict.set("average_wealth", 0.0_f32);
        }

        dict
    }

    /// Returns current residential, commercial, and industrial demand values (-100 to 100).
    pub fn get_demand_stats_internal(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("residential", self.demand.residential);
        dict.set("commercial", self.demand.commercial);
        dict.set("industrial", self.demand.industrial);
        dict
    }
}
